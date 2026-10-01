//! Reading (never writing) credentials the provider CLIs keep in the OS keyring.

/// A generic-password item from the macOS login Keychain, via `/usr/bin/security`. The first
/// read asks the user to allow access, so it waits at most [`KEYCHAIN_TIMEOUT`] for an answer.
#[cfg(target_os = "macos")]
pub fn keychain(service: &str, account: Option<&str>) -> Result<Option<String>, String> {
    let mut command = std::process::Command::new("/usr/bin/security");
    command.args(["find-generic-password", "-s", service, "-w"]);
    if let Some(account) = account {
        command.args(["-a", account]);
    }
    match crate::child::run(command, None, |_| false, KEYCHAIN_TIMEOUT) {
        Ok(out) => {
            let secret = out.stdout.trim().to_owned();
            Ok((out.status.success() && !secret.is_empty()).then_some(secret))
        }
        Err(crate::child::Error::TimedOut) => {
            Err("The Keychain didn't answer. Allow Portal Workspaces access when it asks, then refresh.".into())
        }
        Err(crate::child::Error::Spawn(err) | crate::child::Error::Wait(err)) => {
            Err(format!("Couldn't read the Keychain: {err}"))
        }
    }
}

/// How long a Keychain read may wait, including for the user to answer its access prompt.
#[cfg(target_os = "macos")]
const KEYCHAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

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
