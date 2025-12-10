use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use std::net::{IpAddr, SocketAddr};

use crate::ErrorObject;

static X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");

/// ClientIpError
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClientIpError {
    /// ConnectInfo is missing.
    MissingConnectInfo,
    /// IP is not a valid ascii string.
    InvalidXForwardedForEncoding,
    /// IP string can't be parsed as IpAddr.
    InvalidXForwardedForParse,
}

impl ClientIpError {
    /// Converts Self to ErrorObject.
    pub fn to_error_object(self) -> ErrorObject {
        let err_msg = format!("IP address error: {self:?}");
        ErrorObject {
            status: StatusCode::BAD_REQUEST,
            message: err_msg,
            details: Default::default(),
        }
    }
}

/// GetIPResult
#[derive(Clone, Debug, Copy)]
pub struct GetIPResult {
    /// Ip address or Error.
    pub maybe_ip: Result<IpAddr, ClientIpError>,
}

/// Get the original sender's IP address.
pub fn get_client_ip(
    headers: HeaderMap<HeaderValue>,
    connect_info: Option<&ConnectInfo<SocketAddr>>,
) -> Result<IpAddr, ClientIpError> {
    // Try X-Forwarded-For first
    if let Some(header_val) = headers.get(&X_FORWARDED_FOR) {
        let header_str = header_val.to_str().map_err(|e| {
            tracing::error!(error = %e, "x-forwarded-for is not a valid ascii string.");
            ClientIpError::InvalidXForwardedForEncoding
        })?;

        // Take the first IP in the comma-separated chain
        let first_ip_str = header_str
            .split(',')
            .next()
            .ok_or({
                tracing::error!("x-forwarded-for can't be parsed as IpAddr.");
                ClientIpError::InvalidXForwardedForParse
            })?
            .trim();

        let ip = first_ip_str.parse::<IpAddr>().map_err(|e| {
            tracing::error!(error = %e, "x-forwarded-for can't be parsed as IpAddr.");
            ClientIpError::InvalidXForwardedForParse
        })?;

        return Ok(ip);
    }

    // Fallback to the socket address from ConnectInfo
    let sock_addr = connect_info.ok_or(ClientIpError::MissingConnectInfo)?;
    Ok(sock_addr.ip())
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4};

    use super::*;

    #[test]
    fn test_get_ip() {
        let frowarded_for_ip = "123.123.123.123";
        let connect_info_ip = Ipv4Addr::new(1, 2, 3, 4);
        let connect_info = ConnectInfo(SocketAddr::V4(SocketAddrV4::new(connect_info_ip, 0)));

        // Test happy case
        {
            let mut headers = HeaderMap::new();
            headers.insert(
                "x-forwarded-for",
                HeaderValue::from_static(frowarded_for_ip),
            );
            let ip = get_client_ip(headers, Some(&connect_info)).unwrap();
            assert_eq!(ip.to_string(), ip.to_string());
        }

        // Verify that the happy-case is case-insensitive.
        {
            let mut headers = HeaderMap::new();
            headers.insert(
                "X-Forwarded-For",
                HeaderValue::from_static(frowarded_for_ip),
            );
            let ip = get_client_ip(headers, Some(&connect_info)).unwrap();
            assert_eq!(frowarded_for_ip.to_string(), ip.to_string());
        }

        // If x-forwarded-for is not set then get the ip from ConnectInfo
        {
            let headers = HeaderMap::new();
            let ip = get_client_ip(headers, Some(&connect_info)).unwrap();
            assert_eq!(connect_info_ip.to_string(), ip.to_string());
        }

        // Many ips in x-forwarded-for
        {
            let many_ips = "123.123.123.123, 223.223.223.223,323.323.323.32";
            let mut headers = HeaderMap::new();
            headers.insert("x-forwarded-for", HeaderValue::from_static(many_ips));
            let ip = get_client_ip(headers, None).unwrap();
            assert_eq!(ip.to_string(), ip.to_string());
        }
    }

    #[test]
    fn test_get_invalid_ip() {
        // Test invalid ip format
        {
            let frowarded_for_ip = "123.123.123.1234";
            let mut headers = HeaderMap::new();
            headers.insert(
                "x-forwarded-for",
                HeaderValue::from_static(frowarded_for_ip),
            );
            let err = get_client_ip(headers, None).unwrap_err();
            assert_eq!(err, ClientIpError::InvalidXForwardedForParse);
        }

        // Test missing ip
        {
            let headers = HeaderMap::new();
            let err = get_client_ip(headers, None).unwrap_err();
            assert_eq!(err, ClientIpError::MissingConnectInfo);
        }

        // Empty IP.
        {
            let empty_ip = "";
            let mut headers = HeaderMap::new();
            headers.insert("x-forwarded-for", HeaderValue::from_static(empty_ip));
            let err = get_client_ip(headers, None).unwrap_err();
            assert_eq!(err, ClientIpError::InvalidXForwardedForParse);
        }
    }
}
