//! Step condition evaluation and retry logic

use crate::models::error::ExecutionError;
use crate::models::types::{StepResult, WhenCondition};
use std::collections::HashMap;
use tracing::warn;

pub struct StepEvaluator;

impl StepEvaluator {
    /// Evaluate if a step should run based on `if` condition
    pub fn evaluate_if_condition(
        condition: &str,
        context: &StepEvaluationContext,
    ) -> Result<bool, ExecutionError> {
        // Simple expression evaluator - can be enhanced with a proper expression parser
        // For now, support simple comparisons: branch == "main", env.VAR == "value", etc.
        
        let condition = condition.trim();
        
        // Handle simple boolean expressions
        if condition == "true" {
            return Ok(true);
        }
        if condition == "false" {
            return Ok(false);
        }
        
        // Handle comparisons: branch == "main"
        if let Some((left, right)) = condition.split_once("==") {
            let left = left.trim();
            let right = right.trim().trim_matches('"').trim_matches('\'');
            
            // Check branch
            if left == "branch" || left == "ref_name" {
                return Ok(context.ref_name == right);
            }
            
            // Check environment variables
            if let Some(var_name) = left.strip_prefix("env.") {
                if let Some(value) = context.env_vars.get(var_name) {
                    return Ok(value == right);
                }
                return Ok(false);
            }
            
            // Check previous step results
            if let Some(step_name) = left.strip_prefix("steps.") {
                if let Some(step_result) = context.previous_steps.get(step_name) {
                    if right == "success" || right == "0" {
                        return Ok(step_result.exit_code == 0);
                    }
                    if right == "failure" || right == "failed" {
                        return Ok(step_result.exit_code != 0);
                    }
                }
                return Ok(false);
            }
        }
        
        // Handle != comparisons
        if let Some((left, right)) = condition.split_once("!=") {
            return Ok(!Self::evaluate_if_condition(
                &format!("{} == {}", left.trim(), right.trim()),
                context,
            )?);
        }
        
        // Default: if we can't evaluate, return false (fail-safe)
        warn!("Unable to evaluate condition: {}, defaulting to false", condition);
        Ok(false)
    }
    
    /// Check if step should run based on `when` condition
    pub fn should_run_when(
        when: &WhenCondition,
        previous_step_failed: bool,
    ) -> bool {
        match when {
            WhenCondition::OnSuccess => !previous_step_failed,
            WhenCondition::OnFailure => previous_step_failed,
            WhenCondition::Always => true,
        }
    }
    
    /// Calculate retry delay based on retry policy
    pub fn calculate_retry_delay(
        retry_policy: &crate::models::types::RetryPolicy,
        attempt: u32,
    ) -> std::time::Duration {
        let delay_secs = (retry_policy.initial_delay_secs as f64)
            * retry_policy.backoff_multiplier.powi(attempt as i32 - 1);
        std::time::Duration::from_secs(delay_secs as u64)
    }
}

#[derive(Debug, Clone)]
pub struct StepEvaluationContext {
    pub ref_name: String,
    pub branch: String,
    pub env_vars: HashMap<String, String>,
    pub previous_steps: HashMap<String, StepResult>,
    pub job_status: String,
}

impl StepEvaluationContext {
    pub fn from_job_context(job: &crate::models::types::JobContext) -> Self {
        Self {
            ref_name: job.repository.ref_name.clone(),
            branch: job.repository.ref_name.clone(),
            env_vars: job.config.global_env.clone(),
            previous_steps: HashMap::new(),
            job_status: "running".to_string(),
        }
    }
    
    pub fn with_previous_steps(mut self, steps: &[StepResult]) -> Self {
        for step in steps {
            self.previous_steps.insert(step.name.clone(), step.clone());
        }
        self
    }
}

