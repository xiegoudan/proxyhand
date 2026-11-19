//! Tiny two-language catalog (English / Simplified Chinese).
//!
//! The UI sends its language as `?lang=`; anything without a preference falls back to the
//! environment (`PROXYHAND_LANG`, then `LANG`/`LC_ALL`), then to English.

/// A supported UI language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Zh,
}

impl Lang {
    /// Parse `en`, `en-US`, `zh`, `zh-CN`, `zh_Hans`, …
    pub fn parse(value: &str) -> Option<Lang> {
        let value = value.trim().to_ascii_lowercase();
        if value.starts_with("zh") {
            Some(Lang::Zh)
        } else if value.starts_with("en") {
            Some(Lang::En)
        } else {
            None
        }
    }

    /// First language tag in an `Accept-Language` header, with a sane `q=` fallback.
    pub fn from_accept_language(header: &str) -> Option<Lang> {
        header
            .split(',')
            .filter_map(|part| {
                let mut bits = part.split(';');
                let tag = bits.next()?.trim();
                let q: f32 = bits
                    .find_map(|p| p.trim().strip_prefix("q="))
                    .and_then(|q| q.parse().ok())
                    .unwrap_or(1.0);
                (q > 0.0).then(|| Lang::parse(tag)).flatten()
            })
            .next()
    }

    /// Locale from the environment (`PROXYHAND_LANG`, then `LC_ALL`/`LANG`).
    pub fn from_env() -> Lang {
        for key in ["PROXYHAND_LANG", "LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(value) = std::env::var(key) {
                if let Some(lang) = Lang::parse(&value) {
                    return lang;
                }
            }
        }
        Lang::En
    }
}

pub fn help(lang: Lang) -> String {
    match lang {
        Lang::En => r#"proxyhand — a web debugging proxy for HTTP, HTTPS, SSE and WebSocket

Usage: proxyhand [options]

Options:
  -p, --port <n>        proxy port (default: 7747; HTTP, HTTPS and the web UI all share it)
  -h, --host <addr>     listen address (default: 127.0.0.1, use 0.0.0.0 for LAN/phone)
      --self-hosts <list> comma-separated names that also mean "this proxy", e.g. a
                          container alias (default: localhost, 127.0.0.1, ::1, hostname, IPs)
  -d, --dir <path>      data dir for the root CA (default: ~/.proxyhand)
      --no-ui           run headless, proxy only
      --verify-tls      verify upstream certificates (default: accept anything);
                          also trusts the OS certificate store, like a browser does
      --no-system-ca    with --verify-tls, trust only the bundled roots
                          (default: bundled roots + the CAs installed in the OS)
      --max-flows <n>   flows kept in memory (default: 3000)
      --max-body <n>    bytes kept per body (default: 8388608)
      --upstream-ca <pem>  extra CA cert(s) trusted for upstream TLS, repeatable
                          (for a CA that is in neither the bundled roots nor the OS store)
      --lang <en|zh>    language for the CLI and the embedded UI (default: from $LANG)
  -v, --verbose         log every request
  -V, --version         print version
      --help            print this help
"#
        .to_string(),
        Lang::Zh => r#"proxyhand —— HTTP / HTTPS / SSE / WebSocket 抓包调试代理

用法: proxyhand [选项]

选项:
  -p, --port <n>        代理端口 (默认 7747；HTTP、HTTPS 和 Web UI 共用这一个端口)
  -h, --host <addr>     监听地址 (默认 127.0.0.1，局域网/手机调试用 0.0.0.0)
      --self-hosts <列表> 逗号分隔、也表示"本代理"的额外主机名，例如容器网络别名
                          (默认: localhost, 127.0.0.1, ::1, 主机名, 本机 IP)
  -d, --dir <路径>      根证书数据目录 (默认 ~/.proxyhand)
      --no-ui           不启动 Web UI，只做代理
      --verify-tls      校验上游证书 (默认接受任何证书)；
                          同时信任操作系统证书库，与浏览器的行为一致
      --no-system-ca    配合 --verify-tls，只信任内置根证书
                          (默认: 内置根证书 + 系统证书库)
      --max-flows <n>   内存中保留的请求数 (默认 3000)
      --max-body <n>    每个请求体/响应体保留的字节数 (默认 8388608)
      --upstream-ca <pem>  额外信任的上游 CA 证书，可重复
                          (用于既不在内置根证书、也不在系统证书库里的私有 CA)
      --lang <en|zh>    命令行与内置界面的语言 (默认由 $LANG 决定)
  -v, --verbose         打印每个请求
  -V, --version         显示版本
      --help            显示本帮助
"#
        .to_string(),
    }
}

#[allow(clippy::too_many_arguments)] // a banner is a long, flat list of facts on purpose
pub fn banner(
    lang: Lang,
    version: &str,
    host: &str,
    port: u16,
    ui: bool,
    ca_path: &str,
    verify: bool,
    system_ca: bool,
    extra_cas: usize,
    roots: usize,
) -> String {
    match lang {
        Lang::En => {
            let mut out = format!(
                "proxyhand v{version}\n\n  HTTP proxy   http://{host}:{port}  (HTTP + HTTPS, same port)\n"
            );
            if ui {
                out.push_str(&format!(
                    "  Web UI       http://{host}:{port}  (same port as the proxy)\n"
                ));
            }
            out.push_str(&format!("  CA cert      {ca_path}\n\n"));
            if verify {
                let store = if system_ca {
                    "os store"
                } else {
                    "os store off"
                };
                let extra = if extra_cas > 0 {
                    format!(", +{extra_cas} extra CA(s)")
                } else {
                    String::new()
                };
                out.push_str(&format!(
                    "  upstream TLS verification ON ({roots} trusted roots: bundled + {store}{extra})\n"
                ));
            } else {
                out.push_str("  upstream TLS verification is OFF (use --verify-tls to enforce)\n");
            }
            out
        }
        Lang::Zh => {
            let mut out = format!(
                "proxyhand v{version}\n\n  HTTP 代理    http://{host}:{port}  (HTTP + HTTPS 同端口)\n"
            );
            if ui {
                out.push_str(&format!(
                    "  Web 界面     http://{host}:{port}  (与代理同端口)\n"
                ));
            }
            out.push_str(&format!("  根证书       {ca_path}\n\n"));
            if verify {
                let store = if system_ca {
                    "系统证书库"
                } else {
                    "不读系统证书库"
                };
                let extra = if extra_cas > 0 {
                    format!("，另有 {extra_cas} 个额外 CA")
                } else {
                    String::new()
                };
                out.push_str(&format!(
                    "  上游 TLS 校验已开启 (内置根证书 + {store}{extra})\n"
                ));
            } else {
                out.push_str("  上游 TLS 校验已关闭 (加 --verify-tls 开启)\n");
            }
            out
        }
    }
}

pub fn first_run_hint(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "First run? Install the CA so HTTPS can be decrypted:",
        Lang::Zh => "首次使用？先安装根证书才能解密 HTTPS：",
    }
}

pub fn shutting_down(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "shutting down…",
        Lang::Zh => "正在退出…",
    }
}

/// Platform-specific hint for making the system trust the generated CA.
pub fn trust_instructions(lang: Lang, ca_path: &str) -> Vec<String> {
    let win = cfg!(windows);
    let mac = cfg!(target_os = "macos");
    match lang {
        Lang::En => {
            if win {
                vec![
                    format!("Windows: certutil -addstore -f \"ROOT\" \"{ca_path}\""),
                    "or double-click the file and import it into \"Trusted Root Certification Authorities\"."
                        .to_string(),
                    format!("Node/undici clients ignore the OS store: set NODE_EXTRA_CA_CERTS={ca_path}"),
                ]
            } else if mac {
                vec![
                    format!(
                        "macOS: sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain \"{ca_path}\""
                    ),
                    "Firefox uses its own store: Settings → Privacy → Certificates → Import."
                        .to_string(),
                    format!(
                        "Node/undici clients ignore the OS store: set NODE_EXTRA_CA_CERTS={ca_path}"
                    ),
                ]
            } else {
                vec![
                    format!("Debian/Ubuntu: sudo cp \"{ca_path}\" /usr/local/share/ca-certificates/proxyhand.crt && sudo update-ca-certificates"),
                    format!("Fedora/Arch:  sudo cp \"{ca_path}\" /etc/pki/trust/anchors/ && sudo update-ca-trust"),
                    "Firefox uses its own store: Settings → Privacy → Certificates → View Certificates → Import."
                        .to_string(),
                    format!("Node/undici clients ignore the OS store: set NODE_EXTRA_CA_CERTS={ca_path}"),
                ]
            }
        }
        Lang::Zh => {
            if win {
                vec![
                    format!("Windows: certutil -addstore -f \"ROOT\" \"{ca_path}\""),
                    "或双击该文件，导入到「受信任的根证书颁发机构」。".to_string(),
                    format!("Node/undici 客户端不读系统证书库：设置 NODE_EXTRA_CA_CERTS={ca_path}"),
                ]
            } else if mac {
                vec![
                    format!(
                        "macOS: sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain \"{ca_path}\""
                    ),
                    "Firefox 使用自己的证书库：设置 → 隐私 → 证书 → 查看证书 → 导入。".to_string(),
                    format!("Node/undici 客户端不读系统证书库：设置 NODE_EXTRA_CA_CERTS={ca_path}"),
                ]
            } else {
                vec![
                    format!(
                        "Debian/Ubuntu: sudo cp \"{ca_path}\" /usr/local/share/ca-certificates/proxyhand.crt && sudo update-ca-certificates"
                    ),
                    format!(
                        "Fedora/Arch:  sudo cp \"{ca_path}\" /etc/pki/trust/anchors/ && sudo update-ca-trust"
                    ),
                    "Firefox 使用自己的证书库：设置 → 隐私 → 证书 → 查看证书 → 导入。".to_string(),
                    format!("Node/undici 客户端不读系统证书库：设置 NODE_EXTRA_CA_CERTS={ca_path}"),
                ]
            }
        }
    }
}

pub fn note_tls_fallback(lang: Lang, reason: &str) -> String {
    match lang {
        Lang::En => format!(
            "upstream certificate was not verified ({reason}) — its CA is in neither proxyhand's bundled roots nor the OS trust store; pass --upstream-ca <pem>"
        ),
        Lang::Zh => format!(
            "上游证书未通过校验（{reason}）—— 其 CA 既不在 proxyhand 内置根证书里，也不在系统证书库中；请用 --upstream-ca <pem> 指定"
        ),
    }
}

pub fn note_tunnel_not_tls(lang: Lang) -> String {
    match lang {
        Lang::En => "not TLS: bytes were tunneled without inspection".to_string(),
        Lang::Zh => "不是 TLS：已按原始字节隧道转发，未做解析".to_string(),
    }
}

pub fn note_body_truncated(lang: Lang) -> String {
    match lang {
        Lang::En => {
            "body capture was capped (--max-body): the stored bytes are incomplete".to_string()
        }
        Lang::Zh => "响应体超出抓取上限（--max-body）：已保存的字节不完整".to_string(),
    }
}

pub fn upstream_error(lang: Lang, err: &str) -> String {
    match lang {
        Lang::En => format!("proxyhand: upstream error: {err}"),
        Lang::Zh => format!("proxyhand: 上游错误：{err}"),
    }
}

pub fn header_timeout(lang: Lang, secs: u64) -> String {
    match lang {
        Lang::En => format!("upstream sent no response headers within {secs}s"),
        Lang::Zh => format!("上游在 {secs} 秒内没有返回响应头"),
    }
}

pub fn body_stalled(lang: Lang, secs: u64) -> String {
    match lang {
        Lang::En => format!("upstream body stalled for {secs}s"),
        Lang::Zh => format!("上游响应体已闲置 {secs} 秒"),
    }
}

pub fn loop_detected(lang: Lang) -> String {
    match lang {
        Lang::En => {
            "proxyhand: request loop detected — add this hostname to your proxy bypass list."
                .to_string()
        }
        Lang::Zh => "proxyhand: 检测到请求回环 —— 请把该主机名加入代理绕过列表。".to_string(),
    }
}

pub fn flow_not_found(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "flow not found",
        Lang::Zh => "请求不存在",
    }
}

pub fn no_route(lang: Lang, method: &str, path: &str) -> String {
    match lang {
        Lang::En => format!("no route for {method} {path}"),
        Lang::Zh => format!("没有匹配的路由：{method} {path}"),
    }
}

pub fn body_too_large(lang: Lang, limit: usize) -> String {
    match lang {
        Lang::En => format!("proxyhand: request body larger than {limit} bytes, not buffered"),
        Lang::Zh => format!("proxyhand: 请求体超过 {limit} 字节上限，未做缓冲"),
    }
}

pub fn cannot_read_ca(lang: Lang, file: &str, err: &str) -> String {
    match lang {
        Lang::En => format!("cannot read --upstream-ca \"{file}\": {err}"),
        Lang::Zh => format!("无法读取 --upstream-ca \"{file}\"：{err}"),
    }
}
