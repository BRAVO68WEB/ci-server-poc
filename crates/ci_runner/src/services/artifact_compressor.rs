//! Artifact collection and compression service
//! Collects files/folders matching patterns and compresses them into tar.gz and zip

use crate::models::error::ExecutionError;
use crate::models::types::ArtifactInfo;
use flate2::write::GzEncoder;
use flate2::Compression;
use glob::Pattern;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use tar::Builder;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tracing::{info, warn};
use zip::write::{SimpleFileOptions, ZipWriter};

pub struct ArtifactCompressor;

impl ArtifactCompressor {
    /// Collect artifacts matching patterns and compress them
    /// Returns both individual files and compressed archives (tar.gz, zip)
    pub async fn collect_and_compress(
        workspace_path: &Path,
        patterns: &[String],
        job_id: uuid::Uuid,
    ) -> Result<Vec<ArtifactInfo>, ExecutionError> {
        let mut artifacts = Vec::new();

        // Collect all matching files
        let mut collected_files = Vec::new();
        for pattern_str in patterns {
            let pattern = Pattern::new(pattern_str)
                .map_err(|e| ExecutionError::ConfigError(format!("Invalid artifact pattern {}: {}", pattern_str, e)))?;

            Self::collect_matching_files(workspace_path, workspace_path, &pattern, &mut collected_files).await?;
        }

        if collected_files.is_empty() {
            warn!(patterns = ?patterns, "No files matched artifact patterns");
            return Ok(artifacts);
        }

        info!(count = collected_files.len(), "Collected {} files for compression", collected_files.len());

        // Add each individual file as an artifact
        for file_path in &collected_files {
            if file_path.is_file() {
                let relative = file_path.strip_prefix(workspace_path)
                    .map_err(|e| ExecutionError::IoError(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Path error: {}", e),
                    )))?;
                let relative_str = relative.to_string_lossy().replace('\\', "/");
                
                let file_name = file_path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&relative_str)
                    .to_string();
                
                let artifact_info = Self::create_artifact_info(file_path, workspace_path, &file_name).await?;
                artifacts.push(artifact_info);
            }
        }

        info!(individual_files = artifacts.len(), "Added individual file artifacts");

        // Create artifacts directory for compressed archives
        let artifacts_dir = workspace_path.join(".artifacts");
        fs::create_dir_all(&artifacts_dir).await
            .map_err(ExecutionError::IoError)?;

        // Generate base name for compressed artifacts
        let base_name = format!("artifacts-{}", job_id);

        // Create tar.gz archive
        let tar_gz_path = artifacts_dir.join(format!("{}.tar.gz", base_name));
        Self::create_tar_gz(&collected_files, workspace_path, &tar_gz_path).await?;
        let tar_gz_info = Self::create_artifact_info(&tar_gz_path, workspace_path, &format!("{}.tar.gz", base_name)).await?;
        artifacts.push(tar_gz_info);

        // Create zip archive
        let zip_path = artifacts_dir.join(format!("{}.zip", base_name));
        Self::create_zip(&collected_files, workspace_path, &zip_path).await?;
        let zip_info = Self::create_artifact_info(&zip_path, workspace_path, &format!("{}.zip", base_name)).await?;
        artifacts.push(zip_info);

        info!(
            tar_gz = %tar_gz_path.display(), 
            zip = %zip_path.display(), 
            total_artifacts = artifacts.len(),
            "Created compressed artifacts and collected individual files"
        );
        Ok(artifacts)
    }

    async fn collect_matching_files(
        base: &Path,
        current: &Path,
        pattern: &Pattern,
        files: &mut Vec<PathBuf>,
    ) -> Result<(), ExecutionError> {
        if !current.exists() {
            return Ok(());
        }

        let metadata = fs::metadata(current).await
            .map_err(ExecutionError::IoError)?;

        if metadata.is_file() {
            let relative = current.strip_prefix(base)
                .map_err(|e| ExecutionError::IoError(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Path error: {}", e),
                )))?;

            let relative_str = relative.to_string_lossy().replace('\\', "/");
            if pattern.matches(&relative_str) {
                files.push(current.to_path_buf());
            }
        } else if metadata.is_dir() {
            let mut entries = fs::read_dir(current).await
                .map_err(ExecutionError::IoError)?;

            while let Some(entry) = entries.next_entry().await
                .map_err(ExecutionError::IoError)? {
                Box::pin(Self::collect_matching_files(base, &entry.path(), pattern, files)).await?;
            }
        }

        Ok(())
    }

    async fn create_tar_gz(
        files: &[PathBuf],
        workspace_path: &Path,
        output_path: &Path,
    ) -> Result<(), ExecutionError> {
        let file = fs::File::create(output_path).await
            .map_err(ExecutionError::IoError)?;
        let file = file.into_std().await;

        let encoder = GzEncoder::new(file, Compression::default());
        let mut tar = Builder::new(encoder);

        for file_path in files {
            let relative = file_path.strip_prefix(workspace_path)
                .map_err(|e| ExecutionError::IoError(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Path error: {}", e),
                )))?;

            let relative_str = relative.to_string_lossy().replace('\\', "/");
            
            if file_path.is_file() {
                let mut file = fs::File::open(file_path).await
                    .map_err(ExecutionError::IoError)?;
                let mut contents = Vec::new();
                file.read_to_end(&mut contents).await
                    .map_err(ExecutionError::IoError)?;

                let mut header = tar::Header::new_gnu();
                header.set_path(&relative_str)
                    .map_err(|e| ExecutionError::IoError(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Tar header error: {}", e),
                    )))?;
                header.set_size(contents.len() as u64);
                header.set_cksum();

                tar.append(&header, contents.as_slice())
                    .map_err(|e| ExecutionError::IoError(std::io::Error::other(
                        format!("Tar append error: {}", e),
                    )))?;
            } else if file_path.is_dir() {
                tar.append_dir(&relative_str, file_path)
                    .map_err(|e| ExecutionError::IoError(std::io::Error::other(
                        format!("Tar append_dir error: {}", e),
                    )))?;
            }
        }

        tar.finish()
            .map_err(|e| ExecutionError::IoError(std::io::Error::other(
                format!("Tar finish error: {}", e),
            )))?;

        Ok(())
    }

    async fn create_zip(
        files: &[PathBuf],
        workspace_path: &Path,
        output_path: &Path,
    ) -> Result<(), ExecutionError> {
        let file = fs::File::create(output_path).await
            .map_err(ExecutionError::IoError)?;
        let file = file.into_std().await;

        let mut zip: ZipWriter<std::fs::File> = ZipWriter::new(file);

        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        for file_path in files {
            let relative = file_path.strip_prefix(workspace_path)
                .map_err(|e| ExecutionError::IoError(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Path error: {}", e),
                )))?;

            let relative_str = relative.to_string_lossy().replace('\\', "/");

            if file_path.is_file() {
                let mut file = fs::File::open(file_path).await
                    .map_err(ExecutionError::IoError)?;
                let mut contents = Vec::new();
                file.read_to_end(&mut contents).await
                    .map_err(ExecutionError::IoError)?;

                zip.start_file(&relative_str, options)
                    .map_err(|e| ExecutionError::IoError(std::io::Error::other(
                        format!("Zip start_file error: {}", e),
                    )))?;
                zip.write_all(&contents)
                    .map_err(|e| ExecutionError::IoError(std::io::Error::other(
                        format!("Zip write error: {}", e),
                    )))?;
            } else if file_path.is_dir() {
                zip.add_directory(&relative_str, options)
                    .map_err(|e| ExecutionError::IoError(std::io::Error::other(
                        format!("Zip add_directory error: {}", e),
                    )))?;
            }
        }

        zip.finish()
            .map_err(|e| ExecutionError::IoError(std::io::Error::other(
                format!("Zip finish error: {}", e),
            )))?;

        Ok(())
    }

    async fn create_artifact_info(
        path: &Path,
        _workspace_path: &Path,
        name: &str,
    ) -> Result<ArtifactInfo, ExecutionError> {
        let metadata = fs::metadata(path).await
            .map_err(ExecutionError::IoError)?;
        let size = metadata.len();

        // Calculate checksum
        let mut file = fs::File::open(path).await
            .map_err(ExecutionError::IoError)?;
        let mut hasher = DefaultHasher::new();
        let mut buffer = vec![0u8; 8192];
        loop {
            let n = file.read(&mut buffer).await
                .map_err(ExecutionError::IoError)?;
            if n == 0 {
                break;
            }
            buffer[..n].hash(&mut hasher);
        }
        let checksum = format!("{:x}", hasher.finish());

        Ok(ArtifactInfo {
            name: name.to_string(),
            path: path.to_string_lossy().to_string(),
            size,
            checksum,
            url: None,
        })
    }
}

