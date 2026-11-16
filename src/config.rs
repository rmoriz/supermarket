use std::env::VarError;
use std::fmt;
use std::os::unix::ffi::OsStrExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Config {
    pub bucket: String,
    pub region: String,
    pub url_expiry: Option<u64>,
}

/// Process configuration, read from the same variable names the Rails app uses.
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub redis_url: String,
    /// Sidekiq's own Redis (`REDIS_JOBQ_URL`), falling back to `REDIS_URL`.
    pub redis_jobq_url: String,
    pub secret_key_base: String,
    pub fqdn: String,
    pub port: Option<String>,
    pub protocol: String,
    pub api_item_limit: Option<i32>,
    pub features: Vec<String>,
    pub s3: Option<S3Config>,
    pub fieri_url: Option<String>,
    pub fieri_key: Option<String>,
    pub statsd_host: Option<String>,
    pub robots_allow: Option<String>,
    pub robots_disallow: Option<String>,
    pub chef_oauth2_url: Option<String>,
    pub chef_oauth2_app_id: Option<String>,
    pub chef_oauth2_secret: Option<String>,
    /// `CHEF_OAUTH2_VERIFY_SSL=false` turns certificate checks off for the
    /// id server, which is what `initializers/omniauth.rb` does.
    pub chef_oauth2_verify_ssl: bool,
    /// `ALLOWED_HOSTS`, consulted only with `DISABLE_HOST_HEADER_ATTACK`
    /// (`config/environments/production.rb`).
    pub allowed_hosts: Vec<String>,
    pub github_key: Option<String>,
    pub github_secret: Option<String>,
    pub github_site: Option<String>,
    pub github_authorize_url: Option<String>,
    pub github_access_token_url: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ConfigError {
    pub var: &'static str,
    pub kind: ConfigErrorKind,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigErrorKind {
    Missing,
    NotUnicode,
    IncompleteS3,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ConfigErrorKind::Missing => write!(f, "{} is not set", self.var),
            ConfigErrorKind::NotUnicode => write!(f, "{} is not valid UTF-8", self.var),
            ConfigErrorKind::IncompleteS3 => write!(
                f,
                "{} is set but S3_BUCKET and S3_REGION must both be set",
                self.var
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

fn required(
    env: &dyn Fn(&str) -> Option<String>,
    var: &'static str,
) -> Result<String, ConfigError> {
    match env(var) {
        Some(v) if !v.is_empty() => Ok(v),
        _ => Err(ConfigError {
            var,
            kind: ConfigErrorKind::Missing,
        }),
    }
}

fn optional(env: &dyn Fn(&str) -> Option<String>, var: &str) -> Option<String> {
    env(var).filter(|v| !v.is_empty())
}

/// `ActiveModel::Type::Boolean` accepts these spellings as true.
fn truthy(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "1" | "t" | "true" | "yes" | "on"
    )
}

/// The host allowlist is only built when the attack mitigation is explicitly
/// enabled; otherwise Rails leaves `config.hosts` empty and skips the check.
fn host_allowlist(disable_attack: Option<&str>, allowed: Option<&str>) -> Vec<String> {
    match (disable_attack, allowed) {
        (Some(flag), Some(list)) if truthy(flag) => list
            .split(',')
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

impl Config {
    pub fn from_env() -> Result<Config, ConfigError> {
        Self::from_env_with(&|var| std::env::var(var).ok())
    }

    pub fn from_env_with(env: &dyn Fn(&str) -> Option<String>) -> Result<Config, ConfigError> {
        let database_url = required(env, "DATABASE_URL")?;
        let redis_url = required(env, "REDIS_URL")?;
        let secret_key_base = required(env, "SECRET_KEY_BASE")?;
        let fqdn = required(env, "FQDN")?;

        let api_item_limit = optional(env, "API_ITEM_LIMIT").map(|v| v.parse::<i32>().unwrap_or(0));

        let features = optional(env, "FEATURES")
            .map(|v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let s3 = match (optional(env, "S3_BUCKET"), optional(env, "S3_REGION")) {
            (Some(bucket), Some(region)) => Some(S3Config {
                bucket,
                region,
                url_expiry: optional(env, "S3_URLS_EXPIRE").and_then(|v| v.parse().ok()),
            }),
            (None, None) => None,
            _ => {
                return Err(ConfigError {
                    var: "S3_BUCKET",
                    kind: ConfigErrorKind::IncompleteS3,
                })
            }
        };

        Ok(Config {
            database_url,
            redis_jobq_url: optional(env, "REDIS_JOBQ_URL").unwrap_or_else(|| redis_url.clone()),
            redis_url,
            secret_key_base,
            fqdn,
            port: optional(env, "PORT"),
            protocol: optional(env, "PROTOCOL").unwrap_or_else(|| "http".to_owned()),
            api_item_limit,
            features,
            s3,
            fieri_url: optional(env, "FIERI_URL"),
            fieri_key: optional(env, "FIERI_KEY"),
            statsd_host: match (optional(env, "STATSD_URL"), optional(env, "STATSD_PORT")) {
                (Some(url), Some(port)) => Some(if url.contains(':') {
                    url
                } else {
                    format!("{url}:{port}")
                }),
                _ => None,
            },
            robots_allow: optional(env, "ROBOTS_ALLOW"),
            robots_disallow: optional(env, "ROBOTS_DISALLOW"),
            chef_oauth2_url: optional(env, "CHEF_OAUTH2_URL"),
            chef_oauth2_app_id: optional(env, "CHEF_OAUTH2_APP_ID"),
            chef_oauth2_secret: optional(env, "CHEF_OAUTH2_SECRET"),
            chef_oauth2_verify_ssl: optional(env, "CHEF_OAUTH2_VERIFY_SSL").as_deref()
                != Some("false"),
            allowed_hosts: host_allowlist(
                optional(env, "DISABLE_HOST_HEADER_ATTACK").as_deref(),
                optional(env, "ALLOWED_HOSTS").as_deref(),
            ),
            github_key: optional(env, "GITHUB_KEY"),
            github_secret: optional(env, "GITHUB_SECRET"),
            github_site: optional(env, "GITHUB_CLIENT_OPTION_SITE"),
            github_authorize_url: optional(env, "GITHUB_CLIENT_OPTION_AUTHORIZE_URL"),
            github_access_token_url: optional(env, "GITHUB_CLIENT_OPTION_ACCESS_TOKEN_URL"),
        })
    }

    /// `http://host:port`, the base Rails builds from `FQDN`/`PORT`/`PROTOCOL`.
    pub fn base_url(&self) -> String {
        match &self.port {
            Some(port) if !port.is_empty() => format!("{}://{}:{}", self.protocol, self.fqdn, port),
            _ => format!("{}://{}", self.protocol, self.fqdn),
        }
    }

    pub fn feature_active(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }
}

pub fn from_os_env() -> Result<Config, ConfigError> {
    Config::from_env_with(&|var| {
        std::env::var(var).ok().or_else(|| {
            std::env::var_os(var)
                .map(|v| String::from_utf8(v.as_bytes().to_vec()).unwrap_or_default())
        })
    })
}

#[allow(dead_code)]
fn _assert_var_error(_: VarError) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(pairs: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(&'static str, String)> =
            pairs.iter().map(|(k, v)| (*k, v.to_string())).collect();
        move |var: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == var)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn required_vars_are_required() {
        let err = Config::from_env_with(&|_| None).unwrap_err();
        assert_eq!(err.var, "DATABASE_URL");
        assert_eq!(err.kind, ConfigErrorKind::Missing);
    }

    #[test]
    fn empty_api_item_limit_is_none_and_features_empty() {
        let cfg = Config::from_env_with(&fake(&[
            ("DATABASE_URL", "pg"),
            ("SECRET_KEY_BASE", "s"),
            ("REDIS_URL", "r"),
            ("FQDN", "x.test"),
            ("API_ITEM_LIMIT", ""),
            ("FEATURES", ""),
        ]))
        .unwrap();
        assert_eq!(cfg.api_item_limit, None);
        assert!(cfg.features.is_empty());
        assert!(!cfg.feature_active("chat"));
    }

    #[test]
    fn chat_is_opt_in() {
        let cfg = Config::from_env_with(&fake(&[
            ("DATABASE_URL", "pg"),
            ("SECRET_KEY_BASE", "s"),
            ("REDIS_URL", "r"),
            ("FQDN", "x.test"),
            ("FEATURES", "tools,chat"),
        ]))
        .unwrap();
        assert!(cfg.feature_active("chat"));
    }

    #[test]
    fn features_split_on_comma() {
        let cfg = Config::from_env_with(&fake(&[
            ("DATABASE_URL", "pg"),
            ("SECRET_KEY_BASE", "s"),
            ("REDIS_URL", "r"),
            ("FQDN", "x.test"),
            ("FEATURES", "tools, fieri"),
        ]))
        .unwrap();
        assert_eq!(cfg.features, vec!["tools", "fieri"]);
        assert!(cfg.feature_active("fieri"));
        assert!(!cfg.feature_active("cla"));
    }

    #[test]
    fn partial_s3_config_is_an_error() {
        let err = Config::from_env_with(&fake(&[
            ("DATABASE_URL", "pg"),
            ("SECRET_KEY_BASE", "s"),
            ("REDIS_URL", "r"),
            ("FQDN", "x.test"),
            ("S3_BUCKET", "b"),
        ]))
        .unwrap_err();
        assert_eq!(err.kind, ConfigErrorKind::IncompleteS3);
    }

    #[test]
    fn base_url_omits_empty_port() {
        let cfg = Config::from_env_with(&fake(&[
            ("DATABASE_URL", "pg"),
            ("SECRET_KEY_BASE", "s"),
            ("REDIS_URL", "r"),
            ("FQDN", "x.test"),
            ("PORT", ""),
        ]))
        .unwrap();
        assert_eq!(cfg.base_url(), "http://x.test");
    }

    #[test]
    fn jobq_falls_back_to_redis_url() {
        let cfg = Config::from_env_with(&fake(&[
            ("DATABASE_URL", "pg"),
            ("SECRET_KEY_BASE", "s"),
            ("REDIS_URL", "redis://cache/0"),
            ("REDIS_JOBQ_URL", ""),
            ("FQDN", "x.test"),
        ]))
        .unwrap();
        assert_eq!(cfg.redis_jobq_url, "redis://cache/0");

        let cfg = Config::from_env_with(&fake(&[
            ("DATABASE_URL", "pg"),
            ("SECRET_KEY_BASE", "s"),
            ("REDIS_URL", "redis://cache/0"),
            ("REDIS_JOBQ_URL", "redis://queue/1"),
            ("FQDN", "x.test"),
        ]))
        .unwrap();
        assert_eq!(cfg.redis_jobq_url, "redis://queue/1");
    }

    #[test]
    fn statsd_needs_both_url_and_port() {
        let pairs = [
            ("DATABASE_URL", "pg"),
            ("SECRET_KEY_BASE", "s"),
            ("REDIS_URL", "r"),
            ("FQDN", "x.test"),
        ];
        let with = |extra: &[(&'static str, &'static str)]| {
            let mut all: Vec<(&'static str, &'static str)> = pairs.to_vec();
            all.extend_from_slice(extra);
            Config::from_env_with(&fake(&all)).unwrap().statsd_host
        };
        assert_eq!(with(&[]), None);
        assert_eq!(with(&[("STATSD_URL", "127.0.0.1")]), None);
        assert_eq!(with(&[("STATSD_PORT", "9125")]), None);
        assert_eq!(
            with(&[("STATSD_URL", "127.0.0.1"), ("STATSD_PORT", "9125")]),
            Some("127.0.0.1:9125".to_owned())
        );
    }

    #[test]
    fn oauth_tls_verification_defaults_on() {
        let pairs = [
            ("DATABASE_URL", "pg"),
            ("SECRET_KEY_BASE", "s"),
            ("REDIS_URL", "r"),
            ("FQDN", "x.test"),
        ];
        let verify = |extra: &[(&'static str, &'static str)]| {
            let mut all: Vec<(&'static str, &'static str)> = pairs.to_vec();
            all.extend_from_slice(extra);
            Config::from_env_with(&fake(&all))
                .unwrap()
                .chef_oauth2_verify_ssl
        };
        assert!(verify(&[]));
        assert!(!verify(&[("CHEF_OAUTH2_VERIFY_SSL", "false")]));
        assert!(verify(&[("CHEF_OAUTH2_VERIFY_SSL", "true")]));
    }

    #[test]
    fn host_allowlist_needs_the_switch() {
        let pairs = [
            ("DATABASE_URL", "pg"),
            ("SECRET_KEY_BASE", "s"),
            ("REDIS_URL", "r"),
            ("FQDN", "x.test"),
        ];
        let hosts = |extra: &[(&'static str, &'static str)]| {
            let mut all: Vec<(&'static str, &'static str)> = pairs.to_vec();
            all.extend_from_slice(extra);
            Config::from_env_with(&fake(&all)).unwrap().allowed_hosts
        };
        assert!(hosts(&[("ALLOWED_HOSTS", "a.test,b.test")]).is_empty());
        assert!(hosts(&[("DISABLE_HOST_HEADER_ATTACK", "true")]).is_empty());
        assert_eq!(
            hosts(&[
                ("DISABLE_HOST_HEADER_ATTACK", "1"),
                ("ALLOWED_HOSTS", " a.test , b.test "),
            ]),
            vec!["a.test", "b.test"]
        );
    }
}
