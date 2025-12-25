//! Prometheus metrics collection

use prometheus::{
    register_counter_vec, register_histogram_vec, register_histogram, register_int_gauge, 
    CounterVec, HistogramVec, Histogram, IntGauge,
};
use std::sync::OnceLock;

static METRICS: OnceLock<Metrics> = OnceLock::new();

pub struct Metrics {
    pub job_total: CounterVec,
    pub job_duration: HistogramVec,
    pub job_active: IntGauge,
    pub job_queue_wait: Histogram,
    pub errors_total: CounterVec,
    pub resource_cpu_usage: Histogram,
    pub resource_memory_usage: Histogram,
}

impl Metrics {
    pub fn init() -> &'static Self {
        METRICS.get_or_init(|| {
            Metrics {
                job_total: register_counter_vec!(
                    "ci_job_total",
                    "Total number of jobs processed",
                    &["status"]
                )
                .unwrap(),
                job_duration: register_histogram_vec!(
                    "ci_job_duration_seconds",
                    "Job execution duration in seconds",
                    &["status"],
                    vec![1.0, 5.0, 10.0, 30.0, 60.0, 300.0, 600.0, 1800.0, 3600.0]
                )
                .unwrap(),
                job_active: register_int_gauge!(
                    "ci_job_active",
                    "Number of currently active jobs"
                )
                .unwrap(),
                job_queue_wait: register_histogram!(
                    "ci_job_queue_wait_seconds",
                    "Time jobs wait in queue before execution",
                    vec![0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0]
                )
                .unwrap(),
                errors_total: register_counter_vec!(
                    "ci_errors_total",
                    "Total number of errors",
                    &["type"]
                )
                .unwrap(),
                resource_cpu_usage: register_histogram!(
                    "ci_resource_cpu_usage",
                    "CPU usage per job",
                    vec![0.1, 0.5, 1.0, 2.0, 4.0, 8.0]
                )
                .unwrap(),
                resource_memory_usage: register_histogram!(
                    "ci_resource_memory_usage_bytes",
                    "Memory usage per job in bytes",
                    vec![
                        1024.0 * 1024.0,      // 1MB
                        10.0 * 1024.0 * 1024.0, // 10MB
                        100.0 * 1024.0 * 1024.0, // 100MB
                        1024.0 * 1024.0 * 1024.0, // 1GB
                        4.0 * 1024.0 * 1024.0 * 1024.0, // 4GB
                    ]
                )
                .unwrap(),
            }
        })
    }

    pub fn record_job_start(&self) {
        self.job_active.inc();
    }

    pub fn record_job_complete(&self, status: &str, duration_secs: f64) {
        self.job_total.with_label_values(&[status]).inc();
        self.job_duration
            .with_label_values(&[status])
            .observe(duration_secs);
        self.job_active.dec();
    }

    pub fn record_queue_wait(&self, wait_secs: f64) {
        self.job_queue_wait.observe(wait_secs);
    }

    pub fn record_error(&self, error_type: &str) {
        self.errors_total.with_label_values(&[error_type]).inc();
    }

    pub fn record_resource_usage(&self, cpu: f64, memory_bytes: f64) {
        self.resource_cpu_usage.observe(cpu);
        self.resource_memory_usage.observe(memory_bytes);
    }
}

pub fn get_metrics() -> &'static Metrics {
    Metrics::init()
}

