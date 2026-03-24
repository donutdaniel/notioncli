use std::collections::BTreeMap;
use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

pub const APP_NAME: &str = "notioncli";
pub const LEGACY_APP_NAME: &str = "notion-cli";
pub const DEFAULT_API_VERSION: &str = "2026-03-11";
pub const DEFAULT_API_BASE_URL: &str = "https://api.notion.com";

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub credentials_file: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        if let Ok(raw) =
            env::var("NOTIONCLI_CONFIG_DIR").or_else(|_| env::var("NOTION_CLI_CONFIG_DIR"))
        {
            return Ok(Self::from_base(PathBuf::from(raw)));
        }

        let new_dirs = project_config_dir(APP_NAME)?;
        let legacy_dirs = project_config_dir(LEGACY_APP_NAME)?;

        if !new_dirs.has_local_state() && legacy_dirs.has_local_state() {
            return Ok(legacy_dirs);
        }

        Ok(new_dirs)
    }

    pub fn from_base(base: PathBuf) -> Self {
        let config_file = base.join("config.toml");
        let credentials_file = base.join("credentials.toml");
        Self {
            config_dir: base,
            config_file,
            credentials_file,
        }
    }

    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.config_dir).with_context(|| {
            format!(
                "failed to create config directory at {}",
                self.config_dir.display()
            )
        })
    }

    fn has_local_state(&self) -> bool {
        self.config_file.exists() || self.credentials_file.exists()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(default = "default_api_version")]
    pub api_version: String,
    #[serde(default)]
    pub active_profile: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileMeta>,
}

fn default_api_version() -> String {
    DEFAULT_API_VERSION.to_string()
}

impl Default for ConfigFile {
    fn default() -> Self {
        Self {
            api_version: default_api_version(),
            active_profile: None,
            profiles: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CredentialsFile {
    #[serde(default)]
    profiles: BTreeMap<String, StoredSecret>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileMeta {
    pub auth_type: AuthType,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub bot_id: Option<String>,
    #[serde(default)]
    pub owner_name: Option<String>,
    #[serde(default)]
    pub owner_email: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthType {
    Internal,
}

impl std::fmt::Display for AuthType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "internal")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StoredSecret {
    Internal { token: String },
}

impl StoredSecret {
    pub fn access_token(&self) -> &str {
        let Self::Internal { token } = self;
        token
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSource {
    PersistedProfile,
    Environment,
}

impl SessionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PersistedProfile => "credentials_file",
            Self::Environment => "environment",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeSession {
    pub profile_name: Option<String>,
    pub secret: StoredSecret,
    pub source: SessionSource,
}

impl RuntimeSession {
    pub fn display_name(&self) -> &str {
        self.profile_name.as_deref().unwrap_or("unknown-profile")
    }
}

trait SecretStore: Send + Sync {
    fn write_secret(&self, profile_name: &str, secret: &StoredSecret) -> Result<()>;
    fn read_secret(&self, profile_name: &str) -> Result<StoredSecret>;
    fn delete_secret(&self, profile_name: &str) -> Result<()>;
    fn has_secret(&self, profile_name: &str) -> bool;
}

struct FileSecretStore {
    credentials_file: PathBuf,
}

impl FileSecretStore {
    fn new(credentials_file: PathBuf) -> Self {
        Self { credentials_file }
    }

    fn load_credentials(&self) -> Result<CredentialsFile> {
        if !self.credentials_file.exists() {
            return Ok(CredentialsFile::default());
        }

        let raw = fs::read_to_string(&self.credentials_file).with_context(|| {
            format!(
                "failed to read credentials file {}",
                self.credentials_file.display()
            )
        })?;
        toml::from_str(&raw).with_context(|| {
            format!(
                "failed to parse credentials file {}",
                self.credentials_file.display()
            )
        })
    }

    fn save_credentials(&self, credentials: &CredentialsFile) -> Result<()> {
        if let Some(parent) = self.credentials_file.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create config directory at {}", parent.display())
            })?;
        }

        let raw = toml::to_string_pretty(credentials).context("failed to serialize credentials")?;
        let temp_path = self.credentials_file.with_extension("toml.tmp");
        fs::write(&temp_path, raw).with_context(|| {
            format!(
                "failed to write temporary credentials file {}",
                temp_path.display()
            )
        })?;
        set_private_permissions(&temp_path)?;
        fs::rename(&temp_path, &self.credentials_file).with_context(|| {
            format!(
                "failed to replace credentials file {}",
                self.credentials_file.display()
            )
        })?;
        Ok(())
    }
}

impl SecretStore for FileSecretStore {
    fn write_secret(&self, profile_name: &str, secret: &StoredSecret) -> Result<()> {
        let mut credentials = self.load_credentials()?;
        credentials
            .profiles
            .insert(profile_name.to_string(), secret.clone());
        self.save_credentials(&credentials)
    }

    fn read_secret(&self, profile_name: &str) -> Result<StoredSecret> {
        let credentials = self.load_credentials()?;
        credentials.profiles.get(profile_name).cloned().ok_or_else(|| {
            anyhow!(
                "no persisted credentials found for profile `{profile_name}`; run `notioncli auth login ...` again"
            )
        })
    }

    fn delete_secret(&self, profile_name: &str) -> Result<()> {
        let mut credentials = self.load_credentials()?;
        credentials.profiles.remove(profile_name);
        self.save_credentials(&credentials)
    }

    fn has_secret(&self, profile_name: &str) -> bool {
        self.load_credentials()
            .map(|credentials| credentials.profiles.contains_key(profile_name))
            .unwrap_or(false)
    }
}

#[cfg(test)]
struct InMemorySecretStore {
    secrets: std::sync::Mutex<BTreeMap<String, StoredSecret>>,
}

#[cfg(test)]
impl Default for InMemorySecretStore {
    fn default() -> Self {
        Self {
            secrets: std::sync::Mutex::new(BTreeMap::new()),
        }
    }
}

#[cfg(test)]
impl SecretStore for InMemorySecretStore {
    fn write_secret(&self, profile_name: &str, secret: &StoredSecret) -> Result<()> {
        let mut secrets = self
            .secrets
            .lock()
            .map_err(|_| anyhow!("in-memory test secret store is poisoned"))?;
        secrets.insert(profile_name.to_string(), secret.clone());
        Ok(())
    }

    fn read_secret(&self, profile_name: &str) -> Result<StoredSecret> {
        let secrets = self
            .secrets
            .lock()
            .map_err(|_| anyhow!("in-memory test secret store is poisoned"))?;
        secrets.get(profile_name).cloned().ok_or_else(|| {
            anyhow!(
                "no persisted credentials found for profile `{profile_name}`; run `notioncli auth login ...` again"
            )
        })
    }

    fn delete_secret(&self, profile_name: &str) -> Result<()> {
        let mut secrets = self
            .secrets
            .lock()
            .map_err(|_| anyhow!("in-memory test secret store is poisoned"))?;
        secrets.remove(profile_name);
        Ok(())
    }

    fn has_secret(&self, profile_name: &str) -> bool {
        self.secrets
            .lock()
            .map(|secrets| secrets.contains_key(profile_name))
            .unwrap_or(false)
    }
}

pub struct ConfigStore {
    paths: AppPaths,
    config: ConfigFile,
    secret_store: Arc<dyn SecretStore>,
}

impl ConfigStore {
    pub fn load() -> Result<Self> {
        let paths = AppPaths::discover()?;
        Self::with_secret_store(
            paths.clone(),
            Arc::new(FileSecretStore::new(paths.credentials_file.clone())),
        )
    }

    fn with_secret_store(paths: AppPaths, secret_store: Arc<dyn SecretStore>) -> Result<Self> {
        let config = if paths.config_file.exists() {
            let raw = fs::read_to_string(&paths.config_file).with_context(|| {
                format!("failed to read config file {}", paths.config_file.display())
            })?;
            toml::from_str(&raw).with_context(|| {
                format!(
                    "failed to parse config file {}",
                    paths.config_file.display()
                )
            })?
        } else {
            ConfigFile::default()
        };

        Ok(Self {
            paths,
            config,
            secret_store,
        })
    }

    #[cfg(test)]
    fn with_paths_and_secret_store(
        paths: AppPaths,
        secret_store: Arc<dyn SecretStore>,
    ) -> Result<Self> {
        Self::with_secret_store(paths, secret_store)
    }

    pub fn api_version(&self) -> String {
        env::var("NOTION_API_VERSION")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| self.config.api_version.clone())
    }

    pub fn api_base_url(&self) -> String {
        env::var("NOTION_API_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_API_BASE_URL.to_string())
    }

    pub fn save(&self) -> Result<()> {
        self.paths.ensure()?;
        let raw = toml::to_string_pretty(&self.config).context("failed to serialize config")?;
        fs::write(&self.paths.config_file, raw).with_context(|| {
            format!(
                "failed to write config file {}",
                self.paths.config_file.display()
            )
        })
    }

    pub fn active_profile(&self) -> Option<&str> {
        self.config.active_profile.as_deref()
    }

    pub fn profiles(&self) -> &BTreeMap<String, ProfileMeta> {
        &self.config.profiles
    }

    pub fn get_profile(&self, name: &str) -> Option<&ProfileMeta> {
        self.config.profiles.get(name)
    }

    pub fn put_profile(
        &mut self,
        name: String,
        meta: ProfileMeta,
        secret: &StoredSecret,
    ) -> Result<()> {
        self.secret_store.write_secret(&name, secret)?;
        self.config.profiles.insert(name.clone(), meta);
        self.config.active_profile = Some(name);
        self.save()
    }

    pub fn remove_profile(&mut self, name: &str) -> Result<()> {
        self.config.profiles.remove(name);
        self.secret_store.delete_secret(name)?;

        if self.config.active_profile.as_deref() == Some(name) {
            self.config.active_profile = self
                .config
                .profiles
                .keys()
                .next()
                .map(std::string::ToString::to_string);
        }

        self.save()
    }

    pub fn set_active_profile(&mut self, name: &str) -> Result<()> {
        if !self.config.profiles.contains_key(name) {
            bail!("profile `{name}` does not exist");
        }

        self.config.active_profile = Some(name.to_string());
        self.save()
    }

    pub fn has_persisted_secret(&self, name: &str) -> bool {
        self.secret_store.has_secret(name)
    }

    pub fn resolve_session(&self, explicit_profile: Option<&str>) -> Result<RuntimeSession> {
        let profile = explicit_profile
            .map(str::to_string)
            .or_else(|| self.config.active_profile.clone());

        if let Some(token) = runtime_token_override() {
            return Ok(RuntimeSession {
                profile_name: profile,
                secret: StoredSecret::Internal { token },
                source: SessionSource::Environment,
            });
        }

        let profile = profile.ok_or_else(|| {
            anyhow!(
                "no active profile configured; run `notioncli auth login` or set `NOTION_TOKEN`"
            )
        })?;

        let secret = self.secret_store.read_secret(&profile)?;
        Ok(RuntimeSession {
            profile_name: Some(profile),
            secret,
            source: SessionSource::PersistedProfile,
        })
    }
}

fn project_config_dir(app_name: &str) -> Result<AppPaths> {
    let dirs = ProjectDirs::from("com", "notion", app_name)
        .context("could not determine a platform config directory")?;
    Ok(AppPaths::from_base(dirs.config_dir().to_path_buf()))
}

fn runtime_token_override() -> Option<String> {
    env::var("NOTION_TOKEN")
        .ok()
        .or_else(|| env::var("NOTIONCLI_TOKEN").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn set_private_permissions(_path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(_path, permissions)
            .with_context(|| format!("failed to set private permissions on {}", _path.display()))?;
    }
    Ok(())
}

pub fn slugify_profile_name(raw: &str) -> String {
    let mut output = String::new();
    let mut last_dash = false;

    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !output.is_empty() {
            output.push('-');
            last_dash = true;
        }
    }

    output.trim_matches('-').to_string()
}

pub fn read_text_file(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed to read file {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn slugifies_profile_names() {
        assert_eq!(slugify_profile_name("Team Wiki"), "team-wiki");
        assert_eq!(slugify_profile_name("  QA / Eng  "), "qa-eng");
    }

    #[test]
    fn config_round_trip() -> Result<()> {
        let temp = TempDir::new()?;
        let paths = AppPaths::from_base(temp.path().to_path_buf());
        let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::default());
        let mut store = ConfigStore::with_paths_and_secret_store(paths, secret_store.clone())?;

        let meta = ProfileMeta {
            auth_type: AuthType::Internal,
            workspace_id: Some("abc".into()),
            workspace_name: Some("Workspace".into()),
            bot_id: Some("bot".into()),
            owner_name: Some("Owner".into()),
            owner_email: None,
        };

        store.put_profile(
            "workspace".into(),
            meta.clone(),
            &StoredSecret::Internal {
                token: "access".into(),
            },
        )?;

        let loaded = ConfigStore::with_paths_and_secret_store(
            AppPaths::from_base(temp.path().to_path_buf()),
            secret_store,
        )?;
        let profile = loaded.get_profile("workspace").context("missing profile")?;
        let session = loaded.resolve_session(None)?;

        assert_eq!(profile.workspace_name.as_deref(), Some("Workspace"));
        assert_eq!(loaded.active_profile(), Some("workspace"));
        assert_eq!(session.profile_name.as_deref(), Some("workspace"));
        assert_eq!(session.secret.access_token(), "access");
        assert_eq!(session.source, SessionSource::PersistedProfile);

        Ok(())
    }
}
