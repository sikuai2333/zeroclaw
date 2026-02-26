use crate::config::Config;
use crate::service::{self, InitSystem};
use crate::ServiceCommands;
use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_UPDATE_REPO: &str = "sikuai2333/zeroclaw";

#[derive(Debug, Clone)]
pub struct UpdateOptions {
    pub repo: Option<String>,
    pub tag: Option<String>,
    pub check_only: bool,
    pub restart_service: bool,
    pub force: bool,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct ReleaseResponse {
    tag_name: String,
    html_url: Option<String>,
    assets: Vec<ReleaseAsset>,
}

pub fn run(options: UpdateOptions, config: &Config) -> Result<()> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = options;
        let _ = config;
        bail!("内建更新目前仅支持 Linux");
    }

    #[cfg(target_os = "linux")]
    {
        run_linux_update(options, config)
    }
}

#[cfg(target_os = "linux")]
fn run_linux_update(options: UpdateOptions, config: &Config) -> Result<()> {
    let repo = resolve_repo(options.repo);
    let triple = linux_target_triple()?;
    let binary_name = format!("zeroclaw-{triple}");
    let checksum_name = format!("{binary_name}.sha256");

    let release_url = if let Some(tag) = options.tag.as_deref() {
        format!("https://api.github.com/repos/{repo}/releases/tags/{tag}")
    } else {
        format!("https://api.github.com/repos/{repo}/releases/latest")
    };

    let client = Client::builder()
        .timeout(Duration::from_secs(180))
        .user_agent(format!("zeroclaw-updater/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("Failed to build HTTP client")?;

    println!("🔎 Checking release metadata: {release_url}");
    let release: ReleaseResponse = client
        .get(&release_url)
        .send()
        .context("Failed to request release metadata")?
        .error_for_status()
        .context("Release metadata endpoint returned non-success status")?
        .json()
        .context("Failed to parse release metadata")?;

    let binary_asset = release
        .assets
        .iter()
        .find(|a| a.name == binary_name)
        .with_context(|| format!("Release '{}' missing asset '{}'", release.tag_name, binary_name))?;
    let checksum_asset = release
        .assets
        .iter()
        .find(|a| a.name == checksum_name)
        .with_context(|| {
            format!(
                "Release '{}' missing checksum asset '{}'",
                release.tag_name, checksum_name
            )
        })?;

    let checksum_text = download_text(&client, &checksum_asset.browser_download_url)?;
    let expected_hash = parse_checksum(&checksum_text, &binary_name)?;

    let exe_path = std::env::current_exe().context("Failed to resolve current executable path")?;
    let current_hash = sha256_file(&exe_path)?;
    if options.check_only {
        if current_hash.eq_ignore_ascii_case(&expected_hash) && !options.force {
            println!("ℹ️ Current binary already matches latest release asset checksum");
        } else {
            println!(
                "✅ Update available: tag={} repo={} (current hash differs)",
                release.tag_name, repo
            );
        }
        if let Some(url) = release.html_url.as_deref() {
            println!("   Release: {url}");
        }
        return Ok(());
    }

    if !options.force && current_hash.eq_ignore_ascii_case(&expected_hash) {
        println!("ℹ️ Current binary already matches release asset checksum; skip update");
        return Ok(());
    }

    println!("⬇️  Downloading {}", binary_asset.name);
    let binary_bytes = download_bytes(&client, &binary_asset.browser_download_url)?;
    let downloaded_hash = sha256_hex(&binary_bytes);
    if !downloaded_hash.eq_ignore_ascii_case(&expected_hash) {
        bail!(
            "Checksum mismatch for downloaded binary: expected {}, got {}",
            expected_hash,
            downloaded_hash
        );
    }
    println!("✅ Binary checksum verified");

    if binary_bytes.is_empty() {
        bail!("Downloaded binary is empty");
    }

    let candidate_path = write_candidate_binary(&exe_path, &binary_bytes)?;
    verify_candidate_binary(&candidate_path)?;
    let backup_path = backup_path_for(&exe_path)?;

    println!("🔄 Swapping binary");
    if let Err(err) = replace_binary_atomically(&exe_path, &candidate_path, &backup_path) {
        let _ = fs::remove_file(&candidate_path);
        return Err(err);
    }
    println!("✅ Binary replaced, backup at {}", backup_path.display());

    if options.restart_service && should_restart_managed_service() {
        println!("🔁 Restarting managed service");
        if let Err(restart_err) = restart_managed_service(config) {
            eprintln!("⚠️ Service restart failed, attempting rollback: {restart_err}");
            rollback_binary(&exe_path, &backup_path)
                .context("Rollback failed after restart failure")?;
            let _ = restart_managed_service(config);
            bail!("Update reverted because service restart failed");
        }
        println!("✅ Managed service restarted");
    } else if options.restart_service {
        println!("ℹ️ Managed service not detected; restart skipped");
    } else {
        println!("ℹ️ --no-restart set; service restart skipped");
    }

    println!(
        "🎉 Update completed: repo={} tag={} binary={}",
        repo,
        release.tag_name,
        exe_path.display()
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn resolve_repo(repo_arg: Option<String>) -> String {
    if let Some(repo) = repo_arg {
        let trimmed = repo.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(env_repo) = std::env::var("ZEROCLAW_UPDATE_REPO") {
        let trimmed = env_repo.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    DEFAULT_UPDATE_REPO.to_string()
}

#[cfg(target_os = "linux")]
fn linux_target_triple() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("x86_64-unknown-linux-gnu"),
        "aarch64" => Ok("aarch64-unknown-linux-gnu"),
        "arm" | "armv7" | "armv7l" => Ok("armv7-unknown-linux-gnueabihf"),
        other => bail!("Unsupported Linux architecture: {other}"),
    }
}

#[cfg(target_os = "linux")]
fn download_bytes(client: &Client, url: &str) -> Result<Vec<u8>> {
    let bytes = client
        .get(url)
        .send()
        .with_context(|| format!("Failed to download {url}"))?
        .error_for_status()
        .with_context(|| format!("Download failed with non-success status: {url}"))?
        .bytes()
        .context("Failed to read response body")?;
    Ok(bytes.to_vec())
}

#[cfg(target_os = "linux")]
fn download_text(client: &Client, url: &str) -> Result<String> {
    client
        .get(url)
        .send()
        .with_context(|| format!("Failed to download {url}"))?
        .error_for_status()
        .with_context(|| format!("Download failed with non-success status: {url}"))?
        .text()
        .context("Failed to decode response text")
}

#[cfg(target_os = "linux")]
fn parse_checksum(text: &str, expected_file_name: &str) -> Result<String> {
    let mut fallback_hash: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(hash) = parts.next() else {
            continue;
        };
        if fallback_hash.is_none() {
            fallback_hash = Some(hash.to_ascii_lowercase());
        }
        let maybe_file = parts.next().unwrap_or_default().trim_start_matches('*');
        if maybe_file == expected_file_name {
            return Ok(hash.to_ascii_lowercase());
        }
    }

    fallback_hash.with_context(|| {
        format!(
            "Failed to parse checksum text for expected file '{}'",
            expected_file_name
        )
    })
}

#[cfg(target_os = "linux")]
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(target_os = "linux")]
fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        fs::File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 8192];
    loop {
        let n = file
            .read(&mut buf)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(target_os = "linux")]
fn write_candidate_binary(exe_path: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let parent = exe_path
        .parent()
        .context("Executable path has no parent directory")?;
    let candidate = parent.join(format!(
        ".zeroclaw-update-{}-{}.new",
        unix_ts_ms(),
        std::process::id()
    ));
    fs::write(&candidate, bytes)
        .with_context(|| format!("Failed to write candidate binary {}", candidate.display()))?;
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&candidate)
            .with_context(|| format!("Failed to stat {}", candidate.display()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&candidate, perms)
            .with_context(|| format!("Failed to chmod {}", candidate.display()))?;
    }
    Ok(candidate)
}

#[cfg(target_os = "linux")]
fn verify_candidate_binary(candidate_path: &Path) -> Result<()> {
    let output = Command::new(candidate_path)
        .arg("--help")
        .output()
        .with_context(|| format!("Failed to execute {}", candidate_path.display()))?;
    if !output.status.success() {
        bail!(
            "Candidate binary self-check failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn backup_path_for(exe_path: &Path) -> Result<PathBuf> {
    let parent = exe_path
        .parent()
        .context("Executable path has no parent directory")?;
    let backup_dir = parent.join(".update-backups");
    fs::create_dir_all(&backup_dir)
        .with_context(|| format!("Failed to create backup dir {}", backup_dir.display()))?;
    Ok(backup_dir.join(format!(
        "zeroclaw-{}-{}.bak",
        unix_ts_ms(),
        std::process::id()
    )))
}

#[cfg(target_os = "linux")]
fn replace_binary_atomically(exe_path: &Path, candidate_path: &Path, backup_path: &Path) -> Result<()> {
    fs::rename(exe_path, backup_path).with_context(|| {
        format!(
            "Failed to move current binary to backup: {} -> {}",
            exe_path.display(),
            backup_path.display()
        )
    })?;
    if let Err(err) = fs::rename(candidate_path, exe_path) {
        let _ = fs::rename(backup_path, exe_path);
        bail!(
            "Failed to replace binary: {} -> {} ({err})",
            candidate_path.display(),
            exe_path.display()
        );
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn rollback_binary(exe_path: &Path, backup_path: &Path) -> Result<()> {
    if exe_path.exists() {
        fs::remove_file(exe_path)
            .with_context(|| format!("Failed to remove broken binary {}", exe_path.display()))?;
    }
    fs::rename(backup_path, exe_path).with_context(|| {
        format!(
            "Failed to restore backup binary {} -> {}",
            backup_path.display(),
            exe_path.display()
        )
    })?;
    println!("↩️ Rollback completed: restored {}", exe_path.display());
    Ok(())
}

#[cfg(target_os = "linux")]
fn should_restart_managed_service() -> bool {
    if let Ok(home) = std::env::var("HOME") {
        let user_unit = Path::new(&home).join(".config/systemd/user/zeroclaw.service");
        if user_unit.exists() {
            return true;
        }
    }
    Path::new("/etc/init.d/zeroclaw").exists()
}

#[cfg(target_os = "linux")]
fn restart_managed_service(config: &Config) -> Result<()> {
    service::handle_command(&ServiceCommands::Restart, config, InitSystem::Auto)
}

#[cfg(target_os = "linux")]
fn unix_ts_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}
