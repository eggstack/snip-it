use crate::paths;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn generate_dev_certs(force: bool, out_dir: Option<PathBuf>) -> Result<(), String> {
    let dir = out_dir.unwrap_or_else(paths::cert_dir);
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create cert dir: {}", e))?;

    // Match the names used by the source-tree helper script. The server does
    // not consume these files itself; a TLS-terminating proxy can use them for
    // local development.
    let key_path = dir.join("key.pem");
    let cert_path = dir.join("cert.pem");

    if (key_path.exists() || cert_path.exists()) && !force {
        return Err(format!(
            "Certificates already exist in {}. Use --force to overwrite.",
            dir.display()
        ));
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let temp_key_path = dir.join(format!(".key.pem.{}.{}", std::process::id(), nonce));
    let temp_cert_path = dir.join(format!(".cert.pem.{}.{}", std::process::id(), nonce));
    let cleanup = || {
        let _ = fs::remove_file(&temp_key_path);
        let _ = fs::remove_file(&temp_cert_path);
    };

    let output = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:4096",
            "-nodes",
            "-keyout",
            temp_key_path.to_str().ok_or("Invalid key path")?,
            "-out",
            temp_cert_path.to_str().ok_or("Invalid cert path")?,
            "-days",
            "365",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost,IP:127.0.0.1",
        ])
        .output()
        .map_err(|e| {
            cleanup();
            format!("Failed to run openssl: {}. Is OpenSSL installed?", e)
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        cleanup();
        return Err(format!("openssl failed: {}", stderr));
    }

    // Set permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions(&temp_key_path, fs::Permissions::from_mode(0o600)) {
            cleanup();
            return Err(format!("Failed to set key permissions: {}", e));
        }
        if let Err(e) = fs::set_permissions(&temp_cert_path, fs::Permissions::from_mode(0o644)) {
            cleanup();
            return Err(format!("Failed to set cert permissions: {}", e));
        }
    }

    if !force && (key_path.exists() || cert_path.exists()) {
        cleanup();
        return Err(format!(
            "Certificates already exist in {}. Use --force to overwrite.",
            dir.display()
        ));
    }

    // Only replace existing material after OpenSSL has produced both files, and
    // keep the previous pair recoverable until the new pair is fully installed.
    // Deleting the old files first (as this used to) meant a failure on the
    // second rename left the directory with neither key nor cert — strictly
    // worse than the working material this is meant to protect. Both renames
    // stay inside `dir`, so staging through sibling backup names is cheap.
    let backup_key = dir.join(format!("key.pem.backup.{}", uuid::Uuid::new_v4()));
    let backup_cert = dir.join(format!("cert.pem.backup.{}", uuid::Uuid::new_v4()));
    let mut key_backed_up = false;
    let mut cert_backed_up = false;

    if key_path.exists() {
        fs::rename(&key_path, &backup_key).map_err(|e| {
            cleanup();
            format!("Failed to back up old key: {}", e)
        })?;
        key_backed_up = true;
    }
    if cert_path.exists() {
        fs::rename(&cert_path, &backup_cert).map_err(|e| {
            if key_backed_up {
                let _ = fs::rename(&backup_key, &key_path);
            }
            cleanup();
            format!("Failed to back up old cert: {}", e)
        })?;
        cert_backed_up = true;
    }
    if let Err(e) = fs::rename(&temp_key_path, &key_path) {
        if key_backed_up {
            let _ = fs::rename(&backup_key, &key_path);
        }
        if cert_backed_up {
            let _ = fs::rename(&backup_cert, &cert_path);
        }
        cleanup();
        return Err(format!("Failed to install key: {}", e));
    }
    if let Err(e) = fs::rename(&temp_cert_path, &cert_path) {
        // Drop the half-installed pair, then put the previous material back.
        let _ = fs::remove_file(&key_path);
        if key_backed_up {
            let _ = fs::rename(&backup_key, &key_path);
        }
        if cert_backed_up {
            let _ = fs::rename(&backup_cert, &cert_path);
        }
        cleanup();
        return Err(format!("Failed to install certificate: {}", e));
    }
    // Both files are in place; the backups are no longer needed.
    if key_backed_up {
        let _ = fs::remove_file(&backup_key);
    }
    if cert_backed_up {
        let _ = fs::remove_file(&backup_cert);
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
        fs::write(dir.join("key.pem"), "old").unwrap();

        let result = generate_dev_certs(false, Some(dir));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exist"));
    }

    #[test]
    fn test_generate_dev_certs_force_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("certs");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("key.pem"), "old-key").unwrap();
        fs::write(dir.join("cert.pem"), "old-cert").unwrap();

        // Force should attempt openssl (may fail if openssl not installed)
        let result = generate_dev_certs(true, Some(dir.clone()));
        // If openssl is available, files are regenerated; if not, the old
        // files remain available.
        if result.is_err() {
            assert!(dir.join("key.pem").exists());
        }
    }
}
