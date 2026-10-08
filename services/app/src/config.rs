use anyhow::{Context, Result, bail};

#[derive(Clone, Debug)]
pub struct Config {
    pub app_base_url: String,
    pub app_env: String,
    pub bind_addr: String,
    pub database_url: String,
    pub google_client_id: String,
    pub google_client_secret: String,
    pub google_workspace_domain: String,
    pub google_allowed_emails: Vec<String>,
    pub internal_service_token: String,
    pub slidev_token_secret: String,
    pub asset_local_dir: String,
    pub asset_s3_bucket: Option<String>,
    pub asset_s3_endpoint: Option<String>,
    pub asset_s3_region: Option<String>,
    pub asset_s3_access_key_id: Option<String>,
    pub asset_s3_secret_access_key: Option<String>,
    pub asset_s3_url_style: String,
    pub gemini_moderator_key: Option<String>,
    pub gemini_moderator_model: String,
    pub gemini_assistant_key: Option<String>,
    pub gemini_slide_model: String,
    pub ai_moderation_mode: String,
    pub ai_worker_concurrency: usize,
    pub audience_retention_days: u32,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let _ = dotenvy::dotenv();

        let app_env = env_or(
            "APP_ENV",
            if cfg!(debug_assertions) {
                "development"
            } else {
                "production"
            },
        );
        if !matches!(app_env.as_str(), "development" | "test" | "production") {
            bail!("APP_ENV must be development, test, or production");
        }
        let internal_service_token = env_or("INTERNAL_SERVICE_TOKEN", "development-internal-token");
        let slidev_token_secret = env_or("SLIDEV_TOKEN_SECRET", "development-slidev-token-secret");

        if app_env == "production"
            && (!strong_secret(&internal_service_token)
                || !strong_secret(&slidev_token_secret)
                || internal_service_token == slidev_token_secret)
        {
            bail!(
                "production requires strong INTERNAL_SERVICE_TOKEN and SLIDEV_TOKEN_SECRET values"
            );
        }

        let asset_s3_bucket = optional_env("ASSET_S3_BUCKET");
        let asset_s3_endpoint = optional_env("ASSET_S3_ENDPOINT");
        let asset_s3_access_key_id = optional_env("ASSET_S3_ACCESS_KEY_ID");
        let asset_s3_secret_access_key = optional_env("ASSET_S3_SECRET_ACCESS_KEY");
        let s3_values = [
            asset_s3_bucket.as_ref(),
            asset_s3_endpoint.as_ref(),
            asset_s3_access_key_id.as_ref(),
            asset_s3_secret_access_key.as_ref(),
        ];
        if s3_values.iter().any(|value| value.is_some())
            && s3_values.iter().any(|value| value.is_none())
        {
            bail!(
                "asset S3 configuration must include bucket, endpoint, access key, and secret key"
            );
        }
        if app_env == "production"
            && asset_s3_bucket.is_none()
            && optional_env("ASSET_STORAGE").as_deref() != Some("local")
        {
            bail!(
                "production requires S3 storage or explicit ASSET_STORAGE=local with a persistent volume"
            );
        }

        let google_workspace_domain = env_or("GOOGLE_WORKSPACE_DOMAIN", "").trim().to_lowercase();
        let google_allowed_emails: Vec<String> = env_or("GOOGLE_ALLOWED_EMAILS", "")
            .split(',')
            .map(|email| email.trim().to_lowercase())
            .filter(|email| !email.is_empty())
            .collect();
        if google_workspace_domain.is_empty() && google_allowed_emails.is_empty() {
            bail!(
                "set GOOGLE_WORKSPACE_DOMAIN or GOOGLE_ALLOWED_EMAILS to choose who may create decks"
            );
        }
        if google_workspace_domain.contains(['/', '@', ' ', ':'])
            || google_allowed_emails
                .iter()
                .any(|email| !email.contains('@') || email.contains([' ', '/', ':']))
        {
            bail!("invalid Google account access policy");
        }
        let app_base_url = env_or("APP_BASE_URL", "http://localhost:5173")
            .trim_end_matches('/')
            .to_owned();
        let public_url =
            url::Url::parse(&app_base_url).context("APP_BASE_URL must be an absolute URL")?;
        if !matches!(public_url.scheme(), "http" | "https")
            || public_url.host_str().is_none()
            || !public_url.username().is_empty()
            || public_url.password().is_some()
            || public_url.path() != "/"
            || public_url.query().is_some()
            || public_url.fragment().is_some()
            || (app_env == "production" && public_url.scheme() != "https")
        {
            bail!("APP_BASE_URL must be an origin URL; production requires HTTPS");
        }

        let ai_moderation_mode = env_or("AI_MODERATION_MODE", "assist").to_lowercase();
        if !matches!(ai_moderation_mode.as_str(), "assist" | "enforce") {
            bail!("AI_MODERATION_MODE must be either assist or enforce");
        }
        let ai_worker_concurrency = env_or("AI_WORKER_CONCURRENCY", "4")
            .parse::<usize>()
            .context("AI_WORKER_CONCURRENCY must be an integer")?;
        if !(1..=16).contains(&ai_worker_concurrency) {
            bail!("AI_WORKER_CONCURRENCY must be between 1 and 16");
        }

        let bind_addr = env_or(
            "API_BIND_ADDR",
            if app_env == "production" {
                "0.0.0.0:8080"
            } else {
                "127.0.0.1:8080"
            },
        );

        Ok(Self {
            app_base_url,
            app_env,
            bind_addr,
            database_url: env_or(
                "DATABASE_URL",
                "postgresql://postgres:postgres@127.0.0.1:55432/interdeck",
            ),
            google_client_id: optional_env("GOOGLE_CLIENT_ID")
                .context("GOOGLE_CLIENT_ID is required")?,
            google_client_secret: optional_env("GOOGLE_CLIENT_SECRET")
                .context("GOOGLE_CLIENT_SECRET is required")?,
            google_workspace_domain,
            google_allowed_emails,
            internal_service_token,
            slidev_token_secret,
            asset_local_dir: env_or("ASSET_LOCAL_DIR", ".interdeck/assets"),
            asset_s3_bucket,
            asset_s3_endpoint,
            asset_s3_region: optional_env("ASSET_S3_REGION").or_else(|| Some("auto".to_owned())),
            asset_s3_access_key_id,
            asset_s3_secret_access_key,
            asset_s3_url_style: env_or("ASSET_S3_URL_STYLE", "virtual"),
            gemini_moderator_key: optional_env("GEMINI_MODERATOR_KEY"),
            gemini_moderator_model: env_or("GEMINI_MODERATOR_MODEL", "gemini-3.7-flash"),
            gemini_assistant_key: optional_env("GEMINI_ASSISTANT_KEY"),
            gemini_slide_model: env_or("GEMINI_SLIDE_MODEL", "gemini-3.7-flash"),
            ai_moderation_mode,
            ai_worker_concurrency,
            audience_retention_days: env_or("AUDIENCE_DATA_RETENTION_DAYS", "0")
                .parse()
                .context("AUDIENCE_DATA_RETENTION_DAYS must be a nonnegative integer")?,
        })
    }

    pub fn secure_cookies(&self) -> bool {
        self.app_env == "production" || self.app_base_url.starts_with("https://")
    }

    pub fn oauth_callback_url(&self) -> String {
        format!("{}/api/auth/google/callback", self.app_base_url)
    }

    pub fn google_account_allowed(&self, email: &str, hosted_domain: Option<&str>) -> bool {
        (!self.google_workspace_domain.is_empty()
            && hosted_domain
                .is_some_and(|domain| domain.eq_ignore_ascii_case(&self.google_workspace_domain)))
            || self
                .google_allowed_emails
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(email))
    }
}

fn strong_secret(value: &str) -> bool {
    value.len() >= 32
        && value.bytes().all(|byte| byte.is_ascii_graphic())
        && !value.starts_with("development-")
        && !value.contains("replace-me")
        && value
            .bytes()
            .collect::<std::collections::HashSet<_>>()
            .len()
            >= 8
}

fn env_or(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.to_owned())
}

fn optional_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
pub(crate) fn test_config() -> Config {
    Config {
        app_base_url: "http://localhost:5173".to_owned(),
        app_env: "test".to_owned(),
        bind_addr: "127.0.0.1:0".to_owned(),
        database_url: "postgresql://unused".to_owned(),
        google_client_id: "test".to_owned(),
        google_client_secret: "test".to_owned(),
        google_workspace_domain: "example.org".to_owned(),
        google_allowed_emails: vec![],
        internal_service_token: "test".to_owned(),
        slidev_token_secret: "test".to_owned(),
        asset_local_dir: ".interdeck/test-assets".to_owned(),
        asset_s3_bucket: None,
        asset_s3_endpoint: None,
        asset_s3_region: Some("auto".to_owned()),
        asset_s3_access_key_id: None,
        asset_s3_secret_access_key: None,
        asset_s3_url_style: "virtual".to_owned(),
        gemini_moderator_key: None,
        gemini_moderator_model: "gemini-3.7-flash".to_owned(),
        gemini_assistant_key: None,
        gemini_slide_model: "gemini-3.7-flash".to_owned(),
        ai_moderation_mode: "assist".to_owned(),
        ai_worker_concurrency: 1,
        audience_retention_days: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::strong_secret;

    #[test]
    fn production_rejects_empty_short_placeholder_and_repeated_secrets() {
        for value in [
            "",
            "short",
            "replace-me-in-production",
            "development-internal-token",
            &"a".repeat(64),
        ] {
            assert!(!strong_secret(value));
        }
        assert!(strong_secret(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
    }
}
