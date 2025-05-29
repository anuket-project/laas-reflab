pub(crate) fn fqdn_to_hostname_and_domain(fqdn: &str) -> (String, String) {
    let fqdn = fqdn.trim_end_matches('.');
    let mut parts = fqdn.splitn(2, '.');

    let hostname = parts.next().unwrap_or("");
    let domain = parts.next().unwrap_or("");

    (hostname.to_string(), domain.to_string())
}

pub(crate) fn hostname_and_domain_to_fqdn(hostname: &str, domain: &str) -> String {
    format!(
        "{}.{}",
        hostname.trim_end_matches('.'),
        domain.trim_end_matches('.')
    )
}
