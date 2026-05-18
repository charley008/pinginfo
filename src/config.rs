use std::env;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub bind: SocketAddr,
    pub database_path: PathBuf,
    pub retention_days: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliOptions {
    pub host: Option<IpAddr>,
    pub port: Option<u16>,
    pub show_help: bool,
    pub show_version: bool,
}

impl AppConfig {
    pub fn from_env(cli: &CliOptions) -> anyhow::Result<Self> {
        Self::from_values(
            env::var("PINGINFO_BIND").ok(),
            env::var("PINGINFO_DB").ok(),
            env::var("PINGINFO_RETENTION_DAYS").ok(),
            cli,
        )
    }

    fn from_values(
        bind: Option<String>,
        database_path: Option<String>,
        retention_days: Option<String>,
        cli: &CliOptions,
    ) -> anyhow::Result<Self> {
        let base_bind: SocketAddr = bind
            .unwrap_or_else(|| "0.0.0.0:18080".into())
            .parse()?;
        let bind = SocketAddr::new(
            cli.host.unwrap_or(base_bind.ip()),
            cli.port.unwrap_or(base_bind.port()),
        );
        let database_path = database_path
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data/pinginfo.db"));
        let retention_days = retention_days
            .and_then(|value| value.parse().ok())
            .unwrap_or(30);
        Ok(Self {
            bind,
            database_path,
            retention_days,
        })
    }
}

impl CliOptions {
    pub fn parse() -> anyhow::Result<Self> {
        Self::parse_from(std::env::args().skip(1))
    }

    fn parse_from<I>(args: I) -> anyhow::Result<Self>
    where
        I: IntoIterator<Item = String>,
    {
        let mut options = Self::default();
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--host" => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("missing value for {arg}"))?;
                    options.host = Some(value.parse()?);
                }
                "-p" | "--port" => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("missing value for {arg}"))?;
                    options.port = Some(value.parse()?);
                }
                "--help" => options.show_help = true,
                "-V" | "--version" => options.show_version = true,
                _ => anyhow::bail!("unknown argument: {arg}"),
            }
        }

        Ok(options)
    }

    pub fn print_usage() {
        println!("Usage: pinginfo [-h HOST] [-p PORT] [--host HOST] [--port PORT] [--version]");
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::{AppConfig, CliOptions};

    #[test]
    fn defaults_listen_on_all_interfaces() {
        let config = AppConfig::from_values(None, None, None, &CliOptions::default()).unwrap();

        assert_eq!(config.bind.to_string(), "0.0.0.0:18080");
        assert_eq!(config.database_path, std::path::PathBuf::from("data/pinginfo.db"));
        assert_eq!(config.retention_days, 30);
    }

    #[test]
    fn explicit_values_override_defaults() {
        let config = AppConfig::from_values(
            Some("0.0.0.0:8080".into()),
            Some("/app/data/pinginfo.db".into()),
            Some("7".into()),
            &CliOptions::default(),
        )
        .unwrap();

        assert_eq!(config.bind.to_string(), "0.0.0.0:8080");
        assert_eq!(
            config.database_path,
            std::path::PathBuf::from("/app/data/pinginfo.db")
        );
        assert_eq!(config.retention_days, 7);
    }

    #[test]
    fn cli_host_and_port_override_environment_bind() {
        let cli = CliOptions {
            host: Some("127.0.0.1".parse().unwrap()),
            port: Some(18080),
            ..CliOptions::default()
        };
        let config = AppConfig::from_values(
            Some("0.0.0.0:8080".into()),
            None,
            None,
            &cli,
        )
        .unwrap();

        assert_eq!(config.bind.to_string(), "127.0.0.1:18080");
    }

    #[test]
    fn cli_port_reuses_host_from_environment_bind() {
        let cli = CliOptions {
            host: None,
            port: Some(19090),
            ..CliOptions::default()
        };
        let config = AppConfig::from_values(
            Some("0.0.0.0:8080".into()),
            None,
            None,
            &cli,
        )
        .unwrap();

        assert_eq!(config.bind.to_string(), "0.0.0.0:19090");
    }

    #[test]
    fn parses_short_host_and_port_arguments() {
        let options = CliOptions::parse_from(vec![
            "-h".to_string(),
            "127.0.0.1".to_string(),
            "-p".to_string(),
            "18080".to_string(),
        ])
        .unwrap();

        assert_eq!(
            options,
            CliOptions {
                host: Some("127.0.0.1".parse::<IpAddr>().unwrap()),
                port: Some(18080),
                ..CliOptions::default()
            }
        );
    }

    #[test]
    fn rejects_unknown_arguments() {
        let error = CliOptions::parse_from(vec!["--bind".to_string(), "0.0.0.0".to_string()])
            .unwrap_err();

        assert!(error.to_string().contains("unknown argument"));
    }

    #[test]
    fn parses_version_flag() {
        let options = CliOptions::parse_from(vec!["--version".to_string()]).unwrap();

        assert!(options.show_version);
        assert!(!options.show_help);
        assert_eq!(options.host, None);
        assert_eq!(options.port, None);
    }
}
