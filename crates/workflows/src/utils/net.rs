/// Validates if a given string is a valid IP address.
/// The function uses the `std::net::IpAddr` to parse the string.
///
/// # Arguments
/// * `ip` - The string to validate as IP address.
///
/// # Returns
///
/// Returns `true` if the string is a valid IP address, otherwise `false`.
///
/// # Examples
/// ```
/// assert_eq!(validate_ip("192.168.0.1"), true);
/// assert_eq!(validate_ip("notanip"), false);
/// ``
pub fn validate_ip(ip: &str) -> bool {
    ip.parse::<std::net::IpAddr>().is_ok()
}

/// Validates if a given string is a valid Fully Qualified Domain Name (FQDN) and returns the reason for failure if any.
///
/// # Arguments
///
/// * `fqdn` - The string to validate as FQDN.
///
/// # Returns
///
/// Returns `Ok(true)` if the string is a valid FQDN, otherwise `Err(String)` with a message explaining why it failed.
///
/// # Examples
///
/// ```
/// assert_eq!(validate_fqdn("example.com"), Ok(true));
/// assert_eq!(validate_fqdn("invalid_domain"), Err(String::from("Domain must contain at least two parts")));
/// ```
pub fn validate_fqdn(fqdn: &str) -> Result<(), String> {
    let parts: Vec<&str> = fqdn.split('.').collect();

    if validate_ip(fqdn) {
        return Err("IP address is not a valid FQDN".to_string());
    }

    // basic checks
    if parts.len() < 2 {
        return Err("Domain must contain at least two parts".to_string());
    }
    if fqdn.ends_with('.') || fqdn.starts_with('.') {
        return Err("Domain must not start or end with a dot".to_string());
    }

    // check each part of the domain
    for part in parts {
        if part.is_empty() {
            return Err("Domain parts must not be empty".to_string());
        }
        if part.len() > 63 {
            return Err("Each part of the domain must not exceed 63 characters".to_string());
        }
        if !part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(
                "Domain parts must only consist of alphanumeric characters or hyphens".to_string(),
            );
        }
        if part.starts_with('-') || part.ends_with('-') {
            return Err("Domain parts must not start or end with a hyphen".to_string());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_ip() {
        // valid IP addresses
        assert!(validate_ip("192.168.0.1"));
        assert!(validate_ip("255.255.255.255"));
        assert!(validate_ip("0.0.0.0"));
        assert!(validate_ip("::1")); // IPv6 loopback
        assert!(validate_ip("2001:0db8:85a3:0000:0000:8a2e:0370:7334")); // IPv6 example

        // invalid IP addresses
        assert!(!validate_ip("256.256.256.256")); // out of range
        assert!(!validate_ip("ilomxq74903b1.opnfv.iol.unh.edu")); // obviously not an IP
        assert!(!validate_ip("192.168.0.256")); // one segment out of range
        assert!(!validate_ip("192.168.0")); // incomplete
        assert!(!validate_ip(":::1")); // invalid IPv6
        assert!(!validate_ip("notanip")); // not an IP
    }

    #[test]
    fn test_validate_fqdn() {
        // valid FQDNs
        assert_eq!(validate_fqdn("example.com"), Ok(()));
        assert_eq!(validate_fqdn("www.example.com"), Ok(()));
        assert_eq!(validate_fqdn("sub-domain.example.com"), Ok(()));
        assert_eq!(validate_fqdn("example.co.uk"), Ok(())); // second-level domain
        assert_eq!(validate_fqdn("a.com"), Ok(())); // minimal valid FQDN
        assert_eq!(validate_fqdn("ilomxq74903b1.opnfv.iol.unh.edu"), Ok(())); // real FQDN

        // invalid FQDNs
        assert!(validate_fqdn("example..com").is_err()); // double dots
        assert!(validate_fqdn(".example.com").is_err()); // starts with a dot
        assert!(validate_fqdn("example.com.").is_err()); // ends with a dot
        assert!(validate_fqdn("example").is_err()); // single label
        assert!(validate_fqdn("example_com").is_err()); // underscore not allowed
        assert!(validate_fqdn("example.-com").is_err()); // starts with a dash
        assert!(validate_fqdn("example.com-").is_err()); // ends with a dash
        assert!(validate_fqdn("example..com").is_err()); // empty label due to consecutive dots
        assert!(validate_fqdn("ex*ample.com").is_err()); // invalid character
        assert!(validate_fqdn("123").is_err()); // numeric only is not valid FQDN
        assert!(validate_fqdn("192.168.1.1").is_err()); // ipv4 is not a fqdn
        assert!(validate_fqdn("2001:0db8:85a3:0000:0000:8a2e:0370:7334").is_err());
        // ipv6 is not a fqdn
    }
}
