use crate::error::{AppError, Result};

const SERVICE: &str = "dev.hi5.app";
const ACCOUNT: &str = "github-token";

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| AppError::Auth(e.to_string()))
}

pub fn store_token(token: &str) -> Result<()> {
    entry()?
        .set_password(token)
        .map_err(|e| AppError::Auth(e.to_string()))
}

pub fn load_token() -> Option<String> {
    entry().ok()?.get_password().ok()
}

/// Deletes the stored manual token from the Keychain.
///
/// Unused, and the only surviving `allow(dead_code)`: hi5's sign-out
/// (`Settings::signed_out`) is deliberately its own state and leaves
/// every stored credential where it is -- the CLI's session and this
/// Keychain entry alike -- so nothing calls this. Kept because it is the
/// exact inverse of `store_token` and would be wrong to write from
/// scratch (`NoEntry` being success, in particular, is easy to get
/// wrong) should a "forget the pasted token" affordance ever be wanted.
#[allow(dead_code)]
pub fn clear_token() -> Result<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(AppError::Auth(e.to_string())),
    }
}
