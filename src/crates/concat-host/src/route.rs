// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Which network proxy a request goes through, if any: the one the
//! environment names, or else the one the system's network settings name -
//! the proxy a browser on the same machine is already using. (The smaller
//! copies of media that playback reads are `proxy.rs`, and unrelated.)
//!
//! Concat went straight to the network whatever the machine was set up to
//! do, so a person whose traffic goes through a proxy - a company's
//! gateway, or a tool like Clash or v2rayN set as the system proxy, which is
//! how much of mainland China reaches GitHub and Hugging Face at all - saw
//! every download fail or crawl while the browser beside it worked (#517).
//!
//! The order is curl's and Chrome's together. A proxy named in the
//! environment (`HTTPS_PROXY`, `ALL_PROXY`, with `NO_PROXY`) wins, since
//! someone set it on purpose. Without one, the system's settings: on
//! Windows the per-user settings every browser reads (Settings > Network >
//! Proxy, or Internet Options), a proxy script (PAC) included, which
//! WinHTTP runs for us; on macOS the active network service's proxies, as
//! `scutil --proxy` prints them. Automatic discovery (WPAD) is left alone:
//! it is on by default on Windows, nearly never answers, and costs seconds
//! a request when it does not. A Mac's proxy script is not run either; a
//! machine set up that way is told so in the log. Linux and the phones go
//! by the environment alone.
//!
//! Loopback addresses never go through a proxy, whatever the settings say.

/// A proxy for one request: what ureq connects through, and how to name it
/// in a message without the password it may carry.
#[derive(Clone, Debug)]
pub(crate) struct Route {
    pub(crate) proxy: ureq::Proxy,
    /// "127.0.0.1:7890", or "socks5 127.0.0.1:1080".
    pub(crate) shown: String,
}

/// The proxy a request for `url` should go through: `None` to go directly.
pub(crate) fn for_url(url: &str) -> Option<Route> {
    let (scheme, host) = target(url)?;
    if is_loopback(&host) {
        return None;
    }
    let chosen = match from_env(&scheme, &host, &|name| std::env::var(name).ok()) {
        Some(said) => said,
        None => system(url, &scheme, &host),
    }?;
    match ureq::Proxy::new(&chosen) {
        Ok(proxy) => Some(Route {
            proxy,
            shown: shown(&chosen),
        }),
        Err(error) => {
            log::warn!("network: ignoring the proxy {}: {error}", shown(&chosen));
            None
        }
    }
}

/// The scheme and host of `url`, lowercased, without a port, a user or
/// IPv6's brackets.
fn target(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        bracketed.split(']').next()?
    } else {
        authority.split(':').next()?
    };
    (!host.is_empty()).then(|| (scheme.to_ascii_lowercase(), host.to_ascii_lowercase()))
}

fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host.ends_with(".localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// What the environment says for `host`: `None` when it names no proxy at
/// all, so the system's settings are asked; `Some(None)` when it names one
/// and `NO_PROXY` exempts this host.
///
/// Lowercase `http_proxy` only, as curl reads it: an uppercase
/// `HTTP_PROXY` can be set by a web server's request header (httpoxy).
fn from_env(
    scheme: &str,
    host: &str,
    var: &dyn Fn(&str) -> Option<String>,
) -> Option<Option<String>> {
    let names: &[&str] = if scheme == "https" {
        &["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
    } else {
        &["http_proxy", "ALL_PROXY", "all_proxy"]
    };
    let named = names
        .iter()
        .filter_map(|name| var(name))
        .find(|value| !value.trim().is_empty())?;
    let exempt = ["NO_PROXY", "no_proxy"]
        .iter()
        .filter_map(|name| var(name))
        .any(|list| no_proxy_matches(&list, host));
    if exempt {
        return Some(None);
    }
    Some(normalise(&named, "http"))
}

/// Whether `NO_PROXY` exempts `host`: a comma-separated list where `*` is
/// everything, `example.com` and `.example.com` are that domain and every
/// name under it, and an address or a CIDR block is itself.
fn no_proxy_matches(list: &str, host: &str) -> bool {
    list.split([',', ' ', ';'])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            let entry = entry.to_ascii_lowercase();
            if entry == "*" {
                return true;
            }
            if entry.contains('/') {
                return in_block(&entry, host);
            }
            // A port on an entry narrows it to that port; every request
            // here is to the scheme's own, so the host is what matters.
            let entry = entry.split(':').next().unwrap_or_default();
            let domain = entry.trim_start_matches("*.").trim_start_matches('.');
            host == domain || host.ends_with(&format!(".{domain}"))
        })
}

/// `entry` ("host:port", "socks5://host:port", "user:pass@host") as a URL
/// ureq takes, with `default` as its scheme when it names none. A proxy
/// written `https://` is taken as a plain HTTP one, which is nearly always
/// what was meant and the only kind ureq speaks; `socks5h` is SOCKS5, which
/// ureq already asks to resolve the name.
fn normalise(entry: &str, default: &str) -> Option<String> {
    let entry = entry.trim().trim_end_matches('/');
    if entry.is_empty() {
        return None;
    }
    let (scheme, rest) = match entry.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => (default.to_owned(), entry),
    };
    let scheme = match scheme.as_str() {
        "http" | "https" => "http",
        "socks" | "socks5" | "socks5h" => "socks5",
        "socks4" => "socks4",
        "socks4a" => "socks4a",
        _ => return None,
    };
    (!rest.is_empty()).then(|| format!("{scheme}://{rest}"))
}

/// A proxy URL as a message names it: no password, and the scheme only
/// when it is not plain HTTP.
fn shown(proxy: &str) -> String {
    let (scheme, rest) = proxy.split_once("://").unwrap_or(("http", proxy));
    let place = rest.rsplit_once('@').map_or(rest, |(_, place)| place);
    if scheme == "http" {
        place.to_owned()
    } else {
        format!("{scheme} {place}")
    }
}

/// The proxy a Windows proxy list names for `scheme`.
///
/// The list is what Internet Options writes and WinHTTP hands back: either
/// one "host:port" for everything, or per-protocol entries,
/// "http=host:port;https=host:port;socks=host:port", where a scheme with no
/// entry of its own goes through the SOCKS one, as Chrome reads it. A
/// script's answer is the same shape: a list to try in order, of which the
/// first is taken.
#[cfg_attr(not(any(test, windows)), allow(dead_code))]
fn windows_pick(list: &str, scheme: &str) -> Option<String> {
    let entries: Vec<&str> = list
        .split([';', ' '])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect();
    if let Some(all) = entries.iter().find(|entry| !entry.contains('=')) {
        return normalise(all, "http");
    }
    let named = |name: &str| {
        entries.iter().find_map(|entry| {
            let (key, value) = entry.split_once('=')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then_some(value.trim())
        })
    };
    // SOCKS as IE spoke it is version 4; 4a hands the server the name, so
    // a poisoned local DNS is not what decides where a request goes.
    named(scheme)
        .and_then(|value| normalise(value, "http"))
        .or_else(|| named("socks").and_then(|value| normalise(value, "socks4a")))
}

/// Whether a bypass list exempts `host`: Windows' ProxyOverride
/// ("*.corp.example;10.*;<local>") or a Mac's exceptions ("*.local",
/// "169.254/16"). `<local>`, like a Mac's "exclude simple hostnames", is
/// any name without a dot.
fn bypassed<'a>(patterns: impl IntoIterator<Item = &'a str>, host: &str) -> bool {
    patterns
        .into_iter()
        .map(str::trim)
        .filter(|pattern| !pattern.is_empty())
        .any(|pattern| {
            let pattern = pattern.to_ascii_lowercase();
            if pattern == "<local>" {
                return !host.contains('.') && !host.contains(':');
            }
            if pattern.contains('/') {
                return in_block(&pattern, host);
            }
            glob(&pattern, host)
        })
}

/// `text` against a pattern where `*` is any run of characters.
fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let tail: Vec<&str> = parts.collect();
    let Some((last, middle)) = tail.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// Whether `host` is an IPv4 address inside `block`: "10.0.0.0/8", or the
/// shortened "169.254/16" a Mac writes.
fn in_block(block: &str, host: &str) -> bool {
    let Ok(address) = host.parse::<std::net::Ipv4Addr>() else {
        return false;
    };
    let Some((base, bits)) = block.split_once('/') else {
        return false;
    };
    let Ok(bits) = bits.trim().parse::<u32>() else {
        return false;
    };
    let mut octets = [0u8; 4];
    for (slot, part) in octets.iter_mut().zip(base.split('.')) {
        match part.parse() {
            Ok(value) => *slot = value,
            Err(_) => return false,
        }
    }
    if bits > 32 {
        return false;
    }
    let mask = if bits == 0 {
        0
    } else {
        u32::MAX << (32 - bits)
    };
    u32::from(address) & mask == u32::from_be_bytes(octets) & mask
}

/// What the system's settings say: the outer `None` for no proxy.
#[cfg(windows)]
fn system(url: &str, scheme: &str, host: &str) -> Option<String> {
    windows::system(url, scheme, host)
}

#[cfg(target_os = "macos")]
fn system(_url: &str, scheme: &str, host: &str) -> Option<String> {
    let output = std::process::Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let settings = Scutil::parse(&String::from_utf8_lossy(&output.stdout));
    if settings.on("ProxyAutoConfigEnable") {
        log::debug!(
            "network: this Mac is set to a proxy script ({}), which Concat does not run",
            settings
                .value("ProxyAutoConfigURLString")
                .unwrap_or_default()
        );
    }
    settings.pick(scheme, host)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn system(_url: &str, _scheme: &str, _host: &str) -> Option<String> {
    None
}

/// `scutil --proxy`'s dictionary: its plain keys, and the exceptions list.
#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
#[derive(Default, Debug)]
struct Scutil {
    values: Vec<(String, String)>,
    exceptions: Vec<String>,
}

#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
impl Scutil {
    /// Reads the dictionary `scutil --proxy` prints:
    ///
    /// ```text
    /// <dictionary> {
    ///   ExceptionsList : <array> {
    ///     0 : *.local
    ///   }
    ///   HTTPSEnable : 1
    ///   HTTPSPort : 7890
    ///   HTTPSProxy : 127.0.0.1
    /// }
    /// ```
    fn parse(text: &str) -> Self {
        let mut out = Scutil::default();
        let mut in_exceptions = false;
        for line in text.lines().map(str::trim) {
            if in_exceptions {
                if line.starts_with('}') {
                    in_exceptions = false;
                } else if let Some((_, value)) = line.split_once(" : ") {
                    out.exceptions.push(value.trim().to_owned());
                }
                continue;
            }
            let Some((key, value)) = line.split_once(" : ") else {
                continue;
            };
            if key == "ExceptionsList" {
                in_exceptions = value.trim().ends_with('{');
                continue;
            }
            out.values
                .push((key.trim().to_owned(), value.trim().to_owned()));
        }
        out
    }

    fn value(&self, key: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    fn on(&self, key: &str) -> bool {
        self.value(key) == Some("1")
    }

    /// The proxy for `scheme` and `host`: the scheme's own, or else the
    /// SOCKS one, unless the exceptions or the simple-hostname rule exempt
    /// the host.
    fn pick(&self, scheme: &str, host: &str) -> Option<String> {
        if bypassed(self.exceptions.iter().map(String::as_str), host)
            || (self.on("ExcludeSimpleHostnames") && !host.contains('.'))
        {
            return None;
        }
        let entry = |prefix: &str, kind: &str| {
            if !self.on(&format!("{prefix}Enable")) {
                return None;
            }
            let server = self.value(&format!("{prefix}Proxy"))?;
            let port = self.value(&format!("{prefix}Port"));
            let place = match port {
                Some(port) => format!("{server}:{port}"),
                None => server.to_owned(),
            };
            normalise(&place, kind)
        };
        let own = if scheme == "https" { "HTTPS" } else { "HTTP" };
        entry(own, "http").or_else(|| entry("SOCKS", "socks5"))
    }
}

#[cfg(windows)]
mod windows {
    //! The per-user proxy settings, as WinHTTP reads them for every
    //! program that asks, and the proxy script they may point at, which
    //! WinHTTP fetches and runs.

    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_AUTOPROXY_CONFIG_URL, WINHTTP_AUTOPROXY_OPTIONS,
        WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WINHTTP_PROXY_INFO, WinHttpCloseHandle,
        WinHttpGetIEProxyConfigForCurrentUser, WinHttpGetProxyForUrl, WinHttpOpen,
        WinHttpSetTimeouts,
    };
    use windows_sys::core::PWSTR;

    /// A string WinHTTP allocated, read and freed.
    fn take(text: PWSTR) -> Option<String> {
        if text.is_null() {
            return None;
        }
        // SAFETY: WinHTTP hands back a NUL-terminated wide string it
        // allocated with GlobalAlloc, ours to free once read.
        let read = unsafe {
            let mut length = 0;
            while *text.add(length) != 0 {
                length += 1;
            }
            let read = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
            GlobalFree(text.cast());
            read
        };
        let read = read.trim().to_owned();
        (!read.is_empty()).then_some(read)
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(super) fn system(url: &str, scheme: &str, host: &str) -> Option<String> {
        let mut config = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
        // SAFETY: the struct is ours; on success its strings are WinHTTP's
        // for us to free, which `take` does.
        if unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut config) } == 0 {
            return None;
        }
        let script = take(config.lpszAutoConfigUrl);
        let server = take(config.lpszProxy);
        let bypass = take(config.lpszProxyBypass);
        if let Some(script) = script {
            match run_script(url, &script) {
                Some(answer) => return answer.and_then(|list| super::windows_pick(&list, scheme)),
                None => log::debug!(
                    "network: the proxy script {script} gave no answer; using the fixed settings"
                ),
            }
        }
        let server = server?;
        if bypass.is_some_and(|list| super::bypassed(list.split([';', ' ', ',']), host)) {
            return None;
        }
        super::windows_pick(&server, scheme)
    }

    /// What the proxy script at `script` says for `url`: `Some(None)` for
    /// directly, `None` when it could not be fetched or run.
    fn run_script(url: &str, script: &str) -> Option<Option<String>> {
        let agent = wide("Concat");
        // SAFETY: plain WinHTTP calls on a session opened and closed here;
        // every pointer passed outlives the call it is passed to.
        unsafe {
            let session = WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_NO_PROXY,
                std::ptr::null(),
                std::ptr::null(),
                0,
            );
            if session.is_null() {
                return None;
            }
            // A script on a host that does not answer should cost seconds,
            // not WinHTTP's minute.
            WinHttpSetTimeouts(session, 5_000, 5_000, 5_000, 5_000);
            let script = wide(script);
            let target = wide(url);
            let mut options = WINHTTP_AUTOPROXY_OPTIONS {
                dwFlags: WINHTTP_AUTOPROXY_CONFIG_URL,
                lpszAutoConfigUrl: script.as_ptr(),
                fAutoLogonIfChallenged: 1,
                ..Default::default()
            };
            let mut info = WINHTTP_PROXY_INFO::default();
            let ok = WinHttpGetProxyForUrl(session, target.as_ptr(), &mut options, &mut info) != 0;
            WinHttpCloseHandle(session);
            if !ok {
                return None;
            }
            let list = take(info.lpszProxy);
            let _ = take(info.lpszProxyBypass);
            // The script said DIRECT, or named a proxy.
            if info.dwAccessType == WINHTTP_ACCESS_TYPE_NO_PROXY {
                Some(None)
            } else {
                Some(list)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn a_url_is_read_down_to_its_host() {
        let read = |url| target(url).map(|(scheme, host)| format!("{scheme} {host}"));
        assert_eq!(
            read("https://github.com/jub0t/Concat/releases"),
            Some("https github.com".into())
        );
        assert_eq!(
            read("http://User:pw@Example.COM:8080/x?y"),
            Some("http example.com".into())
        );
        assert_eq!(read("http://[::1]:7420/"), Some("http ::1".into()));
        assert_eq!(read("not a url"), None);
        assert!(is_loopback("127.0.0.1") && is_loopback("::1") && is_loopback("localhost"));
        assert!(!is_loopback("github.com"));
    }

    #[test]
    fn the_environment_names_a_proxy_and_no_proxy_exempts_hosts() {
        let vars = [("HTTPS_PROXY", "127.0.0.1:7890")];
        assert_eq!(
            from_env("https", "github.com", &env(&vars)),
            Some(Some("http://127.0.0.1:7890".into()))
        );
        // Nothing set: the system is asked instead.
        assert_eq!(from_env("https", "github.com", &env(&[])), None);
        // ALL_PROXY covers both, and a SOCKS one keeps its scheme.
        let vars = [("all_proxy", "socks5h://127.0.0.1:1080")];
        assert_eq!(
            from_env("https", "github.com", &env(&vars)),
            Some(Some("socks5://127.0.0.1:1080".into()))
        );
        // An uppercase HTTP_PROXY is not trusted for plain HTTP (httpoxy).
        let vars = [("HTTP_PROXY", "evil:80")];
        assert_eq!(from_env("http", "example.com", &env(&vars)), None);
        // NO_PROXY, curl's way.
        let vars = [
            ("HTTPS_PROXY", "http://proxy.corp:8080/"),
            (
                "NO_PROXY",
                "localhost, .corp.example,huggingface.co,10.0.0.0/8",
            ),
        ];
        let lookup = env(&vars);
        assert_eq!(from_env("https", "huggingface.co", &lookup), Some(None));
        assert_eq!(from_env("https", "cdn.huggingface.co", &lookup), Some(None));
        assert_eq!(from_env("https", "files.corp.example", &lookup), Some(None));
        assert_eq!(from_env("https", "10.2.3.4", &lookup), Some(None));
        assert_eq!(
            from_env("https", "github.com", &lookup),
            Some(Some("http://proxy.corp:8080".into()))
        );
        assert!(no_proxy_matches("*", "anything.example"));
        assert!(!no_proxy_matches("ub.com", "github.com"));
    }

    #[test]
    fn a_windows_proxy_list_is_read_as_browsers_read_it() {
        // One for everything: what Clash and v2rayN set.
        assert_eq!(
            windows_pick("127.0.0.1:7890", "https"),
            Some("http://127.0.0.1:7890".into())
        );
        // Per protocol, with and without schemes on the entries.
        let list = "http=proxy:80;https=secure.proxy:8443;ftp=ftp.proxy:21";
        assert_eq!(
            windows_pick(list, "https"),
            Some("http://secure.proxy:8443".into())
        );
        assert_eq!(windows_pick(list, "http"), Some("http://proxy:80".into()));
        assert_eq!(
            windows_pick("https=http://p:3128", "https"),
            Some("http://p:3128".into())
        );
        // No entry of its own: the SOCKS one.
        assert_eq!(
            windows_pick("http=proxy:80;socks=127.0.0.1:1080", "https"),
            Some("socks4a://127.0.0.1:1080".into())
        );
        assert_eq!(windows_pick("ftp=ftp.proxy:21", "https"), None);
        // A script's answer: a list, of which the first is taken.
        assert_eq!(
            windows_pick("10.0.0.1:8080; 10.0.0.2:8080", "https"),
            Some("http://10.0.0.1:8080".into())
        );
        assert_eq!(windows_pick("", "https"), None);
    }

    #[test]
    fn a_bypass_list_exempts_what_it_names() {
        let list = "*.corp.example;10.*;<local>;192.168.0.0/16";
        let exempt = |host| bypassed(list.split(';'), host);
        assert!(exempt("files.corp.example"));
        assert!(exempt("10.1.2.3"));
        assert!(exempt("intranet"), "<local> is a name without a dot");
        assert!(exempt("192.168.4.20"));
        assert!(!exempt("github.com"));
        assert!(!exempt("corp.example.evil.com"));
        assert!(bypassed(["169.254/16"], "169.254.1.1"));
        assert!(!bypassed(["169.254/16"], "169.255.1.1"));
        assert!(glob("a*b*c", "aXXbYYc") && !glob("a*b*c", "aXXbYY") && glob("*", ""));
    }

    #[test]
    fn a_macs_proxy_settings_are_read_from_scutil() {
        let printed = "<dictionary> {
  ExceptionsList : <array> {
    0 : *.local
    1 : 169.254/16
  }
  ExcludeSimpleHostnames : 1
  FTPPassive : 1
  HTTPEnable : 1
  HTTPPort : 7890
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7891
  HTTPSProxy : 127.0.0.1
  ProxyAutoConfigEnable : 0
  SOCKSEnable : 1
  SOCKSPort : 7892
  SOCKSProxy : 127.0.0.1
}
";
        let settings = Scutil::parse(printed);
        assert_eq!(settings.exceptions, ["*.local", "169.254/16"]);
        assert_eq!(
            settings.pick("https", "github.com"),
            Some("http://127.0.0.1:7891".into())
        );
        assert_eq!(
            settings.pick("http", "example.com"),
            Some("http://127.0.0.1:7890".into())
        );
        assert_eq!(settings.pick("https", "printer.local"), None);
        assert_eq!(settings.pick("https", "nas"), None, "a simple hostname");
        // HTTPS off and SOCKS on: SOCKS.
        let socks_only = Scutil::parse(
            "<dictionary> {\n  SOCKSEnable : 1\n  SOCKSPort : 1080\n  SOCKSProxy : 10.0.0.9\n}\n",
        );
        assert_eq!(
            socks_only.pick("https", "github.com"),
            Some("socks5://10.0.0.9:1080".into())
        );
        // Nothing on: directly.
        let none = Scutil::parse("<dictionary> {\n  HTTPEnable : 0\n  HTTPSEnable : 0\n}\n");
        assert_eq!(none.pick("https", "github.com"), None);
    }

    #[test]
    fn a_proxy_is_named_without_its_password() {
        assert_eq!(
            shown("http://user:secret@proxy.corp:8080"),
            "proxy.corp:8080"
        );
        assert_eq!(shown("socks5://127.0.0.1:1080"), "socks5 127.0.0.1:1080");
        assert_eq!(normalise("ftp://x:1", "http"), None);
        assert_eq!(
            normalise("https://p:3128", "http"),
            Some("http://p:3128".into())
        );
        // What a request is routed by: loopback never through a proxy.
        assert!(for_url("http://127.0.0.1:9/model.bin").is_none());
    }
}
