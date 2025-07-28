//! Configuration for metrics.

/// Variant of transport supported by metrics sender.
#[derive(Debug, Clone, Copy, derivative::Derivative, schemars::JsonSchema, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[derivative(PartialEq)]
pub enum Transport {
    Tcp,
    Udp,
}

impl std::fmt::Display for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Transport::Tcp => write!(f, "tcp"),
            Transport::Udp => write!(f, "udp"),
        }
    }
}

/// Config of receiving `inputs.socket_listener`
/// <https://www.influxdata.com/blog/telegraf-socket-listener-input-plugin/>
#[derive(Debug, Clone, Copy, derivative::Derivative, schemars::JsonSchema)]
#[derivative(PartialEq)]
pub struct TelegrafSocketConfig {
    /// Transport
    pub transport: Transport,
    /// Socket
    pub addr: std::net::SocketAddr,
}

impl std::fmt::Display for TelegrafSocketConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}://{}", self.transport, self.addr)
    }
}

impl std::str::FromStr for TelegrafSocketConfig {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (transport, rest) = match s.split_once("://") {
            Some((proto_str, addr)) => {
                let transport = match proto_str.to_lowercase().as_str() {
                    "tcp" => Transport::Tcp,
                    "udp" => Transport::Udp,
                    _ => return Err(format!("Unknown transport protocol: {proto_str}")),
                };
                (transport, addr)
            }
            None => (Transport::Udp, s),
        };

        let addr = rest
            .parse::<std::net::SocketAddr>()
            .map_err(|e| format!("Invalid address '{rest}': {e}"))?;

        Ok(TelegrafSocketConfig { transport, addr })
    }
}

// Manually implement Serde for SocketConfig to serialize/deserialize as a single string.
impl serde::Serialize for TelegrafSocketConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Re-use our Display impl
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for TelegrafSocketConfig {
    fn deserialize<D>(deserializer: D) -> Result<TelegrafSocketConfig, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        // Re-use our FromStr impl
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl TelegrafSocketConfig {
    /// UDP socket
    pub const fn udp(addr: std::net::SocketAddr) -> Self {
        Self {
            transport: Transport::Udp,
            addr,
        }
    }

    /// TCP socket
    pub const fn tcp(addr: std::net::SocketAddr) -> Self {
        Self {
            transport: Transport::Tcp,
            addr,
        }
    }
}

/// Configuration of Sovereign monitoring
#[derive(
    Debug, Clone, serde::Deserialize, serde::Serialize, derivative::Derivative, schemars::JsonSchema,
)]
#[derivative(PartialEq)]
pub struct MonitoringConfig {
    /// A UDP or TCP socket where Telegraf has `inputs.socket_listener`, something like `udp://127.0.0.1:8094`.
    /// Localhost is preferred for latency and reliability reasons.
    pub telegraf_address: TelegrafSocketConfig,
    /// Defines how many measurements a rollup node will accumulate before sending it to the Telegraf.
    /// It is expected from the rollup node to produce metrics all the time,
    /// so measurements are buffered by size and not sent by time.
    /// and below 67 KB, which is the maximal UDP packet size.
    /// It also means that if a single serialized metric is larger than this value, a UDP packet will be larger.
    ///
    /// Note: to disable buffering, set this value to `Some(1)`.
    pub max_datagram_size: Option<u32>,
    /// How many metrics are allowed to be in pending state, before new metrics will be dropped.
    /// This is a number of metrics, not serialized bytes.
    /// The total number of bytes to be held in memory might vary per metric + `max_datagram_size`
    pub max_pending_metrics: Option<u32>,
}

impl MonitoringConfig {
    /// Standard telegraf socket listener on UDP port on localhost and default parameters.
    pub const fn standard() -> Self {
        Self::default_on_port(8094)
    }

    /// On a specified localhost port with default parameters.
    pub const fn default_on_port(port: u16) -> Self {
        Self {
            telegraf_address: TelegrafSocketConfig::udp(std::net::SocketAddr::V4(
                std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, port),
            )),
            max_datagram_size: None,
            max_pending_metrics: None,
        }
    }

    pub(crate) fn get_max_datagram_size(&self) -> u32 {
        // https://stackoverflow.com/a/35697810/995270
        // Safe max UDP size.
        // A little bit conservative.
        self.max_datagram_size.unwrap_or(508)
    }

    pub(crate) fn get_max_pending_metrics(&self) -> u32 {
        self.max_pending_metrics.unwrap_or(1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_schema() {
        insta::assert_json_snapshot!(schemars::schema_for!(Transport));
    }

    #[test]
    fn only_udp() {
        let config_s = r#"
            telegraf_address = "udp://127.0.0.1:8094"
        "#;
        let config = toml::from_str::<MonitoringConfig>(config_s).unwrap();
        insta::assert_json_snapshot!(config);
    }

    #[test]
    fn udp_with_other_params() {
        let config_s = r#"
            telegraf_address = "udp://127.0.0.1:8094"
            max_datagram_size = 508
            max_pending_metrics = 100
        "#;
        let config = toml::from_str::<MonitoringConfig>(config_s).unwrap();
        insta::assert_json_snapshot!(config);
    }

    #[test]
    fn only_tcp() {
        let config_s = r#"
            telegraf_address = "tcp://127.0.0.1:8094"
        "#;
        let config = toml::from_str::<MonitoringConfig>(config_s).unwrap();
        insta::assert_json_snapshot!(config);
    }

    #[test]
    fn udp_when_not_specified() {
        let config_s = r#"
            telegraf_address = "127.0.0.1:8094"
        "#;
        let config = toml::from_str::<MonitoringConfig>(config_s).unwrap();
        assert_eq!(config.telegraf_address.transport, Transport::Udp);
        insta::assert_json_snapshot!(config);
    }
}
