//! Reading (never writing) credentials the provider CLIs keep in the OS keyring.

/// A generic-password item from the macOS login Keychain, via `/usr/bin/security`. The first
/// read asks the user to allow access.
#[cfg(target_os = "macos")]
pub fn keychain(service: &str, account: Option<&str>) -> Option<String> {
    let mut args = vec!["find-generic-password", "-s", service, "-w"];
    if let Some(account) = account {
        args.extend(["-a", account]);
    }
    let out = std::process::Command::new("/usr/bin/security")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let secret = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !secret.is_empty()).then_some(secret)
}

/// The Secret Service item (GNOME Keyring, KWallet) with exactly these attributes.
#[cfg(target_os = "linux")]
pub fn secret_service(attributes: &[(&str, &str)]) -> Result<Option<String>, String> {
    use dbus_secret_service::{EncryptionType, SecretService};

    let service = SecretService::connect(EncryptionType::Plain).map_err(|e| format!("No keyring service: {e}"))?;
    let found = service
        .search_items(attributes.iter().copied().collect())
        .map_err(|e| format!("Keyring search failed: {e}"))?;
    let item = match (found.unlocked.into_iter().next(), found.locked.is_empty()) {
        (Some(item), _) => item,
        (None, false) => return Err("The keyring is locked.".into()),
        (None, true) => return Ok(None),
    };
    let secret = item.get_secret().map_err(|e| format!("Couldn't read the keyring item: {e}"))?;
    Ok(String::from_utf8(secret).ok())
}
