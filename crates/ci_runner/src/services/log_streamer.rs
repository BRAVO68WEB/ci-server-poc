use crate::config::LogStreamerConfig;
use crate::models::error::StreamError;
use crate::models::types::{LogEntry, LogLevel};
use chrono::Utc;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::interval;
use tracing::{error, warn};
use uuid::Uuid;

pub struct LogStreamer {
    client: Client,
    buffer: Arc<Mutex<LogBuffer>>,
    config: LogStreamerConfig,
    auth_token: String,
    sequence_map: Arc<Mutex<HashMap<Uuid, u64>>>,
}

struct LogBuffer {
    entries: Vec<LogEntry>,
    size: usize,
    max_size: usize,
}

impl LogBuffer {
    fn new(max_size: usize) -> Self {
        Self {
            entries: Vec::new(),
            size: 0,
            max_size,
        }
    }

    fn push(&mut self, entry: LogEntry) {
        let entry_size = entry.message.len();
        self.entries.push(entry);
        self.size += entry_size;
    }

    fn drain(&mut self) -> Vec<LogEntry> {
        let entries = std::mem::take(&mut self.entries);
        self.size = 0;
        entries
    }

    fn should_flush(&self) -> bool {
        self.size >= self.max_size || !self.entries.is_empty()
    }
}

impl LogStreamer {
    pub fn new(config: LogStreamerConfig, auth_token: String) -> Self {
        let client = Client::builder()
            .timeout(config.git_server.timeout)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            buffer: Arc::new(Mutex::new(LogBuffer::new(config.buffer.size))),
            config,
            auth_token,
            sequence_map: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn send(
        &self,
        job_id: Uuid,
        run_id: Uuid,
        level: LogLevel,
        message: &[u8],
    ) -> Result<(), StreamError> {
        self.send_with_step(job_id, run_id, None, level, message).await
    }

    pub async fn send_with_step(
        &self,
        job_id: Uuid,
        run_id: Uuid,
        step_name: Option<String>,
        level: LogLevel,
        message: &[u8],
    ) -> Result<(), StreamError> {
        let message_str = String::from_utf8_lossy(message).to_string();

        // Get next sequence number
        let sequence = {
            let mut map = self.sequence_map.lock().await;
            let seq = map.entry(job_id).or_insert(0);
            *seq += 1;
            *seq - 1
        };

        let entry = LogEntry {
            job_id,
            run_id,
            timestamp: Utc::now(),
            level,
            step_name,
            message: message_str,
            sequence,
        };

        // Add to buffer
        {
            let mut buffer = self.buffer.lock().await;
            buffer.push(entry.clone());
        }

        // Flush if buffer is full
        let should_flush = {
            let buffer = self.buffer.lock().await;
            buffer.should_flush()
        };

        if should_flush {
            self.flush().await?;
        }

        Ok(())
    }

    pub async fn flush(&self) -> Result<(), StreamError> {
        let entries = {
            let mut buffer = self.buffer.lock().await;
            buffer.drain()
        };

        if entries.is_empty() {
            return Ok(());
        }

        // Group entries by job_id
        let mut grouped: HashMap<Uuid, Vec<LogEntry>> = HashMap::new();
        for entry in entries {
            grouped
                .entry(entry.job_id)
                .or_default()
                .push(entry);
        }

        // Send each group
        for (job_id, job_entries) in grouped {
            let url = format!(
                "{}/api/v1/ci/jobs/{}/logs",
                self.config.git_server.base_url, job_id
            );

            let response = self
                .client
                .post(&url)
                .bearer_auth(&self.auth_token)
                .json(&job_entries)
                .send()
                .await
                .map_err(StreamError::NetworkError)?;

            if !response.status().is_success() {
                warn!(
                    "Failed to send logs for job {}: HTTP {}",
                    job_id,
                    response.status().as_u16()
                );
                return Err(StreamError::HttpError(response.status().as_u16()));
            }
        }

        Ok(())
    }

    pub fn start_background_flusher(self: Arc<Self>) {
        let flush_interval = self.config.buffer.flush_interval;
        tokio::spawn(async move {
            let mut interval = interval(flush_interval);

            loop {
                interval.tick().await;

                if let Err(e) = self.flush().await {
                    error!("Failed to flush logs: {}", e);
                }
            }
        });
    }
}

// Implement the trait for executor integration
impl crate::services::executor::LogStreamerTrait for LogStreamer {
    fn send(&self, job_id: Uuid, run_id: Uuid, step_name: Option<String>, level: crate::models::types::LogLevel, message: &[u8]) {
        // This is a synchronous interface, but we need async
        // We'll use a channel or spawn a task
        let streamer = self.clone();
        let message = message.to_vec();
        tokio::spawn(async move {
            if let Err(e) = streamer.send_with_step(job_id, run_id, step_name, level, &message).await {
                error!("Failed to send log: {}", e);
            }
        });
    }
}

impl Clone for LogStreamer {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            buffer: Arc::clone(&self.buffer),
            config: self.config.clone(),
            auth_token: self.auth_token.clone(),
            sequence_map: Arc::clone(&self.sequence_map),
        }
    }
}
