//! Private host/preview protocol for a requested media change after shutdown.
use std::path::PathBuf;

const MAX_REQUEST_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaRequest {
    InsertCartridge(PathBuf),
    EjectCartridge,
    InstallSd(PathBuf),
}

impl MediaRequest {
    pub fn encode(&self) -> Option<String> {
        let (action, path) = match self {
            Self::InsertCartridge(path) => ("insert-cartridge", Some(path)),
            Self::EjectCartridge => ("eject-cartridge", None),
            Self::InstallSd(path) => ("install-sd", Some(path)),
        };
        let mut result = format!("schema = 1\naction = \"{action}\"\n");
        if let Some(path) = path {
            let path = path.to_str()?;
            if path.is_empty() || path.chars().any(char::is_control) {
                return None;
            }
            result.push_str(&format!("path = {}\n", toml::Value::String(path.into())));
        }
        (result.len() <= MAX_REQUEST_BYTES).then_some(result)
    }

    pub fn decode(bytes: &[u8]) -> Result<Option<Self>, &'static str> {
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err("media request exceeds size limit");
        }
        let text = std::str::from_utf8(bytes).map_err(|_| "media request is not UTF-8")?;
        if text.trim().is_empty() {
            return Ok(None);
        }
        let table = text
            .parse::<toml::Table>()
            .map_err(|_| "media request is malformed")?;
        if table.get("schema").and_then(toml::Value::as_integer) != Some(1)
            || table
                .keys()
                .any(|key| !matches!(key.as_str(), "schema" | "action" | "path"))
        {
            return Err("media request schema is invalid");
        }
        let action = table
            .get("action")
            .and_then(toml::Value::as_str)
            .ok_or("media request action missing")?;
        let path = table.get("path").and_then(toml::Value::as_str);
        if action == "eject-cartridge" {
            return if table.contains_key("path") {
                Err("ejection request contains a path")
            } else {
                Ok(Some(Self::EjectCartridge))
            };
        }
        let path = path
            .filter(|path| !path.is_empty() && !path.chars().any(char::is_control))
            .ok_or("media request path invalid")?;
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("media request path is not absolute");
        }
        match action {
            "insert-cartridge" => Ok(Some(Self::InsertCartridge(path))),
            "install-sd" => Ok(Some(Self::InstallSd(path))),
            _ => Err("media request action unsupported"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_round_trip_preserves_special_path_characters() {
        let path = std::env::temp_dir().join("test games, quoted 'title' #1.cci");
        for request in [
            MediaRequest::InsertCartridge(path.clone()),
            MediaRequest::InstallSd(path),
            MediaRequest::EjectCartridge,
        ] {
            assert_eq!(
                MediaRequest::decode(request.encode().unwrap().as_bytes()),
                Ok(Some(request))
            );
        }
        assert_eq!(MediaRequest::decode(b""), Ok(None));
    }
    #[test]
    fn rejects_ambiguous_or_unexpected_request_data() {
        for value in [
            "schema=1\naction='eject-cartridge'\npath='extra'",
            "schema=1\naction='insert-cartridge'\npath='relative.cci'",
            "schema=1\naction='eject-cartridge'\nextra=1",
            "schema=1\naction='eject-cartridge'\naction='install-sd'",
        ] {
            assert!(MediaRequest::decode(value.as_bytes()).is_err());
        }
        assert!(MediaRequest::decode(&vec![b' '; MAX_REQUEST_BYTES + 1]).is_err());
    }
}
