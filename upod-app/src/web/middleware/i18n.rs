use axum::extract::Request;
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;
use rust_i18n::set_locale;

pub async fn handle_i18n(req: Request, next: Next) -> Response {
    let lang = resolve_language(req.headers());
    set_locale(lang.as_str());
    next.run(req).await
}

fn resolve_language(headers: &HeaderMap) -> String {
    let default_lang = "zh".to_string();

    let Some(header) = headers.get("accept-language").and_then(|v| v.to_str().ok()) else {
        return default_lang;
    };

    // 简易解析: "zh-CN,zh;q=0.9,en;q=0.8" -> ["zh-CN", "zh", "en"]
    let mut langs: Vec<(f32, String)> = header
        .split(',')
        .filter_map(|part| {
            let mut sections = part.split(';');
            let lang = sections.next()?.trim().to_string();
            let q_value = sections
                .next()
                .and_then(|q| q.trim().strip_prefix("q="))
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(1.0);
            Some((q_value, lang))
        })
        .collect();

    // 按权重降序排列
    langs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // 取第一个语言
    langs.first().map(|v| v.1.clone()).unwrap_or(default_lang)
}
