//! A stable app-private device GUID preserves normal server attempt accounting.
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("private identity storage unavailable")]
    Storage,
    #[error("stored device identity is invalid")]
    Invalid,
}
fn valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn read(path: &Path) -> Result<String, IdentityError> {
    let file = File::open(path).map_err(|_| IdentityError::Storage)?;
    let mut value = String::new();
    file.take(129)
        .read_to_string(&mut value)
        .map_err(|_| IdentityError::Invalid)?;
    if !valid(&value) {
        return Err(IdentityError::Invalid);
    }
    Ok(value)
}
pub fn load_or_create(directory: &Path) -> Result<String, IdentityError> {
    if !directory.is_absolute() || !directory.is_dir() {
        return Err(IdentityError::Storage);
    }
    let path = directory.join("atv-device-guid");
    if path.exists() {
        return read(&path);
    }
    let value = atv_auth::generate_device_guid();
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => {
            file.write_all(value.as_bytes())
                .and_then(|_| file.sync_all())
                .map_err(|_| IdentityError::Storage)?;
            Ok(value)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read(&path),
        Err(_) => Err(IdentityError::Storage),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_survives_reload_and_corruption_does_not_reset_attempts() {
        let dir = std::env::temp_dir().join(format!(
            "atv-identity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let first = load_or_create(&dir).unwrap();
        assert_eq!(load_or_create(&dir).unwrap(), first);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.join("atv-device-guid"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        std::fs::write(dir.join("atv-device-guid"), "invalid; cookie=injection").unwrap();
        assert!(matches!(load_or_create(&dir), Err(IdentityError::Invalid)));
        assert_eq!(
            std::fs::read_to_string(dir.join("atv-device-guid")).unwrap(),
            "invalid; cookie=injection"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
