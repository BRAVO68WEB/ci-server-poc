//! Deploy key generation and management for SSH-based Git cloning

use crate::models::error::ExecutionError;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use tokio::process::Command;
use tracing::{info, warn};

const REDIS_PRIVATE_KEY: &str = "ci_runner:deploy_key:private_key";
const REDIS_PUBLIC_KEY: &str = "ci_runner:deploy_key:public_key";
const REDIS_KEY_FINGERPRINT: &str = "ci_runner:deploy_key:fingerprint";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployKey {
    pub public_key: String,
    pub fingerprint: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployKeyPair {
    pub private_key: String,
    pub public_key: String,
    pub fingerprint: String,
}

pub struct DeployKeyManager {
    redis_client: Option<redis::Client>,
    key_dir: PathBuf,
}

impl DeployKeyManager {
    /// Create a new DeployKeyManager
    /// If redis_url is None, keys will only be stored on filesystem
    pub async fn new(redis_url: Option<&str>, key_dir: PathBuf) -> Result<Self, ExecutionError> {
        // Ensure key directory exists
        fs::create_dir_all(&key_dir).await.map_err(ExecutionError::IoError)?;

        let redis_client = if let Some(url) = redis_url {
            Some(
                redis::Client::open(url)
                    .map_err(|e| ExecutionError::ConfigError(format!("Redis connection error: {}", e)))?
            )
        } else {
            None
        };

        Ok(Self {
            redis_client,
            key_dir,
        })
    }

    /// Generate a new SSH key pair using ssh-keygen
    pub async fn generate_key_pair(&self) -> Result<DeployKeyPair, ExecutionError> {
        let temp_key_path = self.key_dir.join("temp_deploy_key");
        let temp_pub_path = self.key_dir.join("temp_deploy_key.pub");

        // Remove existing temp files
        let _ = fs::remove_file(&temp_key_path).await;
        let _ = fs::remove_file(&temp_pub_path).await;

        // Generate ED25519 key (more secure and faster than RSA)
        let output = Command::new("ssh-keygen")
            .args([
                "-t", "ed25519",
                "-f", temp_key_path.to_str().unwrap(),
                "-N", "",  // No passphrase
                "-C", "ci-runner-deploy-key",
            ])
            .output()
            .await
            .map_err(ExecutionError::IoError)?;

        if !output.status.success() {
            return Err(ExecutionError::ConfigError(format!(
                "ssh-keygen failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        // Read generated keys
        let private_key = fs::read_to_string(&temp_key_path)
            .await
            .map_err(ExecutionError::IoError)?;

        let public_key = fs::read_to_string(&temp_pub_path)
            .await
            .map_err(ExecutionError::IoError)?;

        // Get fingerprint
        let fingerprint_output = Command::new("ssh-keygen")
            .args(["-lf", temp_pub_path.to_str().unwrap()])
            .output()
            .await
            .map_err(ExecutionError::IoError)?;

        let fingerprint = String::from_utf8_lossy(&fingerprint_output.stdout)
            .split_whitespace()
            .nth(1)
            .unwrap_or("unknown")
            .to_string();

        // Clean up temp files
        let _ = fs::remove_file(&temp_key_path).await;
        let _ = fs::remove_file(&temp_pub_path).await;

        info!(fingerprint = %fingerprint, "Generated new deploy key");

        Ok(DeployKeyPair {
            private_key: private_key.trim().to_string(),
            public_key: public_key.trim().to_string(),
            fingerprint,
        })
    }

    /// Store key pair in Redis
    pub async fn store_key_pair(&self, key_pair: &DeployKeyPair) -> Result<(), ExecutionError> {
        if let Some(ref client) = self.redis_client {
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| ExecutionError::ConfigError(format!("Redis connection error: {}", e)))?;

            // Store private key
            conn.set::<_, _, ()>(REDIS_PRIVATE_KEY, &key_pair.private_key)
                .await
                .map_err(|e| ExecutionError::ConfigError(format!("Redis set error: {}", e)))?;

            // Store public key
            conn.set::<_, _, ()>(REDIS_PUBLIC_KEY, &key_pair.public_key)
                .await
                .map_err(|e| ExecutionError::ConfigError(format!("Redis set error: {}", e)))?;

            // Store fingerprint
            conn.set::<_, _, ()>(REDIS_KEY_FINGERPRINT, &key_pair.fingerprint)
                .await
                .map_err(|e| ExecutionError::ConfigError(format!("Redis set error: {}", e)))?;

            info!(fingerprint = %key_pair.fingerprint, "Stored deploy key in Redis");
        } else {
            // Fallback: store on filesystem
            let private_path = self.key_dir.join("deploy_key");
            let public_path = self.key_dir.join("deploy_key.pub");

            fs::write(&private_path, &key_pair.private_key)
                .await
                .map_err(ExecutionError::IoError)?;
            
            // Set proper permissions (600) for private key
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&private_path).await.map_err(ExecutionError::IoError)?.permissions();
                perms.set_mode(0o600);
                fs::set_permissions(&private_path, perms).await.map_err(ExecutionError::IoError)?;
            }

            fs::write(&public_path, &key_pair.public_key)
                .await
                .map_err(ExecutionError::IoError)?;

            info!(path = %private_path.display(), "Stored deploy key on filesystem");
        }

        Ok(())
    }

    /// Get the public key info (for display/registration with Git server)
    pub async fn get_public_key(&self) -> Result<Option<DeployKey>, ExecutionError> {
        if let Some(ref client) = self.redis_client {
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| ExecutionError::ConfigError(format!("Redis connection error: {}", e)))?;

            let public_key: Option<String> = conn.get(REDIS_PUBLIC_KEY)
                .await
                .map_err(|e| ExecutionError::ConfigError(format!("Redis get error: {}", e)))?;

            let fingerprint: Option<String> = conn.get(REDIS_KEY_FINGERPRINT)
                .await
                .map_err(|e| ExecutionError::ConfigError(format!("Redis get error: {}", e)))?;

            match (public_key, fingerprint) {
                (Some(pk), Some(fp)) => Ok(Some(DeployKey {
                    public_key: pk,
                    fingerprint: fp,
                    created_at: chrono::Utc::now(), // Note: could store this too
                })),
                _ => Ok(None),
            }
        } else {
            // Fallback: read from filesystem
            let public_path = self.key_dir.join("deploy_key.pub");
            if public_path.exists() {
                let public_key = fs::read_to_string(&public_path)
                    .await
                    .map_err(ExecutionError::IoError)?;

                // Get fingerprint
                let output = Command::new("ssh-keygen")
                    .args(["-lf", public_path.to_str().unwrap()])
                    .output()
                    .await
                    .map_err(ExecutionError::IoError)?;

                let fingerprint = String::from_utf8_lossy(&output.stdout)
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("unknown")
                    .to_string();

                Ok(Some(DeployKey {
                    public_key: public_key.trim().to_string(),
                    fingerprint,
                    created_at: chrono::Utc::now(),
                }))
            } else {
                Ok(None)
            }
        }
    }

    /// Get or create the deploy key, returning the private key path for SSH cloning
    pub async fn get_private_key_path(&self) -> Result<PathBuf, ExecutionError> {
        let key_path = self.key_dir.join("deploy_key");

        // Check if we need to fetch from Redis
        if let Some(ref client) = self.redis_client {
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| ExecutionError::ConfigError(format!("Redis connection error: {}", e)))?;

            let private_key: Option<String> = conn.get(REDIS_PRIVATE_KEY)
                .await
                .map_err(|e| ExecutionError::ConfigError(format!("Redis get error: {}", e)))?;

            if let Some(pk) = private_key {
                // Write to filesystem for SSH to use
                fs::write(&key_path, &pk)
                    .await
                    .map_err(ExecutionError::IoError)?;

                // Set proper permissions (600)
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mut perms = fs::metadata(&key_path).await.map_err(ExecutionError::IoError)?.permissions();
                    perms.set_mode(0o600);
                    fs::set_permissions(&key_path, perms).await.map_err(ExecutionError::IoError)?;
                }

                return Ok(key_path);
            }
        }

        // Check filesystem
        if key_path.exists() {
            return Ok(key_path);
        }

        // No key exists - generate one
        warn!("No deploy key found, generating new one");
        let key_pair = self.generate_key_pair().await?;
        self.store_key_pair(&key_pair).await?;

        // Write to filesystem
        fs::write(&key_path, &key_pair.private_key)
            .await
            .map_err(ExecutionError::IoError)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&key_path).await.map_err(ExecutionError::IoError)?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&key_path, perms).await.map_err(ExecutionError::IoError)?;
        }

        Ok(key_path)
    }

    /// Generate and store a new key pair (for regeneration)
    pub async fn regenerate(&self) -> Result<DeployKey, ExecutionError> {
        let key_pair = self.generate_key_pair().await?;
        self.store_key_pair(&key_pair).await?;

        info!(fingerprint = %key_pair.fingerprint, "Regenerated deploy key");

        Ok(DeployKey {
            public_key: key_pair.public_key,
            fingerprint: key_pair.fingerprint,
            created_at: chrono::Utc::now(),
        })
    }

    /// Check if a deploy key exists
    pub async fn key_exists(&self) -> Result<bool, ExecutionError> {
        if let Some(ref client) = self.redis_client {
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| ExecutionError::ConfigError(format!("Redis connection error: {}", e)))?;

            let exists: bool = conn.exists(REDIS_PRIVATE_KEY)
                .await
                .map_err(|e| ExecutionError::ConfigError(format!("Redis exists error: {}", e)))?;

            return Ok(exists);
        }

        // Filesystem fallback
        Ok(self.key_dir.join("deploy_key").exists())
    }
}

/// Helper to create ServiceAuth from deploy key for the cloner
pub async fn create_ssh_auth(key_manager: &DeployKeyManager) -> Result<crate::services::cloner::ServiceAuth, ExecutionError> {
    let key_path = key_manager.get_private_key_path().await?;
    
    Ok(crate::services::cloner::ServiceAuth {
        token_type: crate::services::cloner::TokenType::SSH { key_path },
        token: secrecy::SecretString::from(""),
        expiry: None,
    })
}

