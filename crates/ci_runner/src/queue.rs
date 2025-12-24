use crate::config::QueueConfig;
use crate::error::ExecutionError;
use crate::types::JobEvent;
use futures_util::StreamExt;
use lapin::{Channel, Connection, ConnectionProperties, options::*, types::FieldTable};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use uuid::Uuid;

pub struct QueueConsumer {
    channel: Arc<Channel>,
    config: QueueConfig,
}

impl QueueConsumer {
    pub async fn new(config: QueueConfig) -> Result<Self, ExecutionError> {
        let username = if let Some(ref username_file) = config.username_file {
            tokio::fs::read_to_string(username_file)
                .await
                .map_err(|e| {
                    ExecutionError::ConfigError(format!("Failed to read username file: {}", e))
                })?
                .trim()
                .to_string()
        } else {
            "guest".to_string()
        };

        let password = if let Some(ref password_file) = config.password_file {
            tokio::fs::read_to_string(password_file)
                .await
                .map_err(|e| {
                    ExecutionError::ConfigError(format!("Failed to read password file: {}", e))
                })?
                .trim()
                .to_string()
        } else {
            "guest".to_string()
        };

        // URL-encode the virtual host (forward slashes become %2F)
        // For AMQP URLs, we need to encode the vhost path
        let vhost_encoded = config.virtual_host.replace('/', "%2F");
        let addr = format!(
            "amqp://{}:{}@{}:{}/{}",
            username, password, config.host, config.port, vhost_encoded
        );

        info!("Connecting to RabbitMQ at {}", config.host);

        let connection = Connection::connect(&addr, ConnectionProperties::default())
            .await
            .map_err(|e| {
                ExecutionError::QueueError(format!("Failed to connect to RabbitMQ: {}", e))
            })?;

        let channel = connection
            .create_channel()
            .await
            .map_err(|e| ExecutionError::QueueError(format!("Failed to create channel: {}", e)))?;

        // Declare queue
        channel
            .queue_declare(
                &config.queue_name,
                QueueDeclareOptions::default(),
                FieldTable::default(),
            )
            .await
            .map_err(|e| ExecutionError::QueueError(format!("Failed to declare queue: {}", e)))?;

        // Declare dead letter queue
        channel
            .queue_declare(
                &config.dead_letter_queue,
                QueueDeclareOptions::default(),
                FieldTable::default(),
            )
            .await
            .map_err(|e| ExecutionError::QueueError(format!("Failed to declare DLQ: {}", e)))?;

        // Set prefetch count
        channel
            .basic_qos(config.prefetch_count, BasicQosOptions::default())
            .await
            .map_err(|e| ExecutionError::QueueError(format!("Failed to set prefetch: {}", e)))?;

        info!("Connected to RabbitMQ queue: {}", config.queue_name);

        Ok(Self {
            channel: Arc::new(channel),
            config,
        })
    }

    pub async fn consume<F, Fut>(&mut self, handler: F) -> Result<(), ExecutionError>
    where
        F: Fn(JobEvent) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ExecutionError>> + Send + 'static,
    {
        let mut consumer = self
            .channel
            .basic_consume(
                &self.config.queue_name,
                "ci-runner",
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await
            .map_err(|e| ExecutionError::QueueError(format!("Failed to create consumer: {}", e)))?;

        info!("Started consuming from queue: {}", self.config.queue_name);

        let handler = Arc::new(handler);
        let channel = Arc::clone(&self.channel);
        let dlq = self.config.dead_letter_queue.clone();

        tokio::spawn(async move {
            let retry_counts: Arc<RwLock<std::collections::HashMap<Uuid, u32>>> =
                Arc::new(RwLock::new(std::collections::HashMap::new()));

            while let Some(delivery_result) = consumer.next().await {
                match delivery_result {
                    Ok(delivery) => {
                        let channel = Arc::clone(&channel);
                        let handler = Arc::clone(&handler);
                        let retry_counts = Arc::clone(&retry_counts);
                        let dlq = dlq.clone();

                        tokio::spawn(async move {
                            match Self::process_delivery(
                                delivery,
                                handler,
                                retry_counts,
                                channel,
                                dlq,
                            )
                            .await
                            {
                                Ok(_) => {}
                                Err(e) => {
                                    error!("Error processing delivery: {}", e);
                                }
                            }
                        });
                    }
                    Err(e) => {
                        error!("Consumer error: {}", e);
                    }
                }
            }
        });

        Ok(())
    }

    async fn process_delivery<F, Fut>(
        delivery: lapin::message::Delivery,
        handler: Arc<F>,
        retry_counts: Arc<RwLock<std::collections::HashMap<Uuid, u32>>>,
        channel: Arc<Channel>,
        dlq: String,
    ) -> Result<(), ExecutionError>
    where
        F: Fn(JobEvent) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = Result<(), ExecutionError>> + Send,
    {
        let job_event: JobEvent = serde_json::from_slice(&delivery.data)
            .map_err(|e| ExecutionError::QueueError(format!("Failed to parse job event: {}", e)))?;

        let job_id = job_event.job_id;
        info!(job_id = %job_id, "Received job event");

        // Execute handler
        match handler(job_event).await {
            Ok(_) => {
                // Acknowledge success
                channel
                    .basic_ack(delivery.delivery_tag, BasicAckOptions::default())
                    .await
                    .map_err(|e| ExecutionError::QueueError(format!("Failed to ACK: {}", e)))?;

                // Remove retry count
                retry_counts.write().await.remove(&job_id);

                Ok(())
            }
            Err(e) => {
                error!(job_id = %job_id, error = %e, "Job execution failed");

                // Check retry count
                let retry_count = {
                    let mut counts = retry_counts.write().await;
                    let count = counts.entry(job_id).or_insert(0);
                    *count += 1;
                    *count
                };

                const MAX_RETRIES: u32 = 5;
                if retry_count >= MAX_RETRIES {
                    // Move to DLQ
                    warn!(job_id = %job_id, "Max retries reached, moving to DLQ");

                    channel
                        .basic_publish(
                            "",
                            &dlq,
                            BasicPublishOptions::default(),
                            &delivery.data,
                            lapin::BasicProperties::default(),
                        )
                        .await
                        .map_err(|e| {
                            ExecutionError::QueueError(format!("Failed to publish to DLQ: {}", e))
                        })?;

                    channel
                        .basic_ack(delivery.delivery_tag, BasicAckOptions::default())
                        .await
                        .map_err(|e| ExecutionError::QueueError(format!("Failed to ACK: {}", e)))?;

                    retry_counts.write().await.remove(&job_id);
                } else {
                    // Reject and requeue with exponential backoff
                    let backoff = std::time::Duration::from_secs(2_u64.pow(retry_count - 1));
                    warn!(job_id = %job_id, retry_count, backoff_secs = backoff.as_secs(), "Requeuing job");

                    tokio::time::sleep(backoff).await;

                    channel
                        .basic_nack(
                            delivery.delivery_tag,
                            BasicNackOptions {
                                requeue: true,
                                multiple: false,
                            },
                        )
                        .await
                        .map_err(|e| {
                            ExecutionError::QueueError(format!("Failed to NACK: {}", e))
                        })?;
                }

                Ok(())
            }
        }
    }
}
