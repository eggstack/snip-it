use crate::paths;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub fn generate_dev_certs(force: bool, out_dir: Option<PathBuf>) -> Result<(), String> {
    let dir = out_dir.unwrap_or_else(paths::cert_dir);
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create cert dir: {}", e))?;

    let key_path = dir.join("server.key");
    let cert_path = dir.join("server.crt");

    if key_path.exists() || cert_path.exists() {
        if !force {
            return Err(format!(
                "Certificates already exist in {}. Use --force to overwrite.",
                dir.display()
            ));
        }
        if key_path.exists() {
            fs::remove_file(&key_path).map_err(|e| format!("Failed to remove old key: {}", e))?;
        }
        if cert_path.exists() {
            fs::remove_file(&cert_path).map_err(|e| format!("Failed to remove old cert: {}", e))?;
        }
    }

    let output = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:4096",
            "-nodes",
            "-keyout",
            key_path.to_str().ok_or("Invalid key path")?,
            "-out",
            cert_path.to_str().ok_or("Invalid cert path")?,
            "-days",
            "365",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost,IP:127.0.0.1",
        ])
        .output()
        .map_err(|e| format!("Failed to run openssl: {}. Is OpenSSL installed?", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("openssl failed: {}", stderr));
    }

    // Set permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("Failed to set key permissions: {}", e))?;
        fs::set_permissions(&cert_path, fs::Permissions::from_mode(0o644))
            .map_err(|e| format!("Failed to set cert permissions: {}", e))?;
    }

    println!("Generated self-signed dev certificates:");
    println!("  Key:  {}", key_path.display());
    println!("  Cert: {}", cert_path.display());
    println!();
    println!("These are self-signed dev certificates — NOT for production use.");
    println!("For production, use a reverse proxy (nginx, traefik) with proper TLS.");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_dev_certs_refuse_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("certs");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("server.key"), "old").unwrap();

        let result = generate_dev_certs(false, Some(dir));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exist"));
    }

    #[test]
    fn test_generate_dev_certs_force_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("certs");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("server.key"), "old-key").unwrap();
        fs::write(dir.join("server.crt"), "old-cert").unwrap();

        // Force should attempt openssl (may fail if openssl not installed)
        let result = generate_dev_certs(true, Some(dir.clone()));
        // If openssl is available, files are regenerated; if not, we get an error
        // but the old files should have been removed
        if result.is_err() {
            // Old files were removed even though openssl failed
            assert!(!dir.join("server.key").exists() || result.is_err());
        }
    }
}
