use std::collections::HashMap;

use bollard::Docker;
use bollard::query_parameters::ListContainersOptions;
use chrono::{TimeZone, Utc};
use serde_qs::axum::QsQuery;
use upod_base::web::r::R;
use upod_base::web::error::WebError;
use crate::core::code::Code;
use super::docker::{SANDBOX_ID_LABEL, resolve_sandbox_id};
use crate::models::sandbox::{ListSandboxesReq, ListSandboxesResp, Sandbox};

/// 列出沙箱列表
///
/// 支持根据状态和元数据过滤。
pub async fn list_sandboxes(QsQuery(req): QsQuery<ListSandboxesReq>) -> R<ListSandboxesResp> {
    // 连接到本地 Docker 守护进程，后续按标签筛选沙箱容器。
    let docker = match Docker::connect_with_local_defaults() {
        Ok(docker) => docker,
        Err(error) => return R::err(docker_connect_error(error)),
    };

    // 默认按沙箱标签过滤，避免把非沙箱容器带入返回结果。
    let mut filters = HashMap::new();
    filters.insert("label".to_string(), vec![SANDBOX_ID_LABEL.to_string()]);

    // 状态过滤直接映射到 Docker 的 status 过滤器，支持多状态并集查询。
    if let Some(states) = req.state
        && !states.is_empty()
    {
        for state in &states {
            filters.entry("status".to_string()).or_insert_with(Vec::new).push(state.clone());
        }
    }

    // 元数据过滤转换为 label=k=v 形式，交给 Docker 侧进行匹配。
    if let Some(metadata) = &req.metadata {
        for (k, v) in metadata {
            filters.entry("label".to_string())
                .or_insert_with(Vec::new)
                .push(format!("{}={}", k, v));
        }
    }

    let options = ListContainersOptions {
        all: true, // 列出所有容器，不仅仅是运行中的
        filters: Some(filters),
        ..Default::default()
    };

    // 调用 Docker API 拉取容器列表。
    let containers = match docker.list_containers(Some(options)).await {
        Ok(containers) => containers,
        Err(error) => return R::err(WebError::BizWithArgs(
            Code::InternalServerError.into(),
            vec![("error".to_string(), error.to_string())],
        )),
    };

    // 规范化 Docker 字段并转换成业务层 Sandbox 结构。
    let mut items = Vec::new();

    for c in &containers {
        let created_at = if let Some(ts) = c.created {
            // Docker 返回的是秒级时间戳
            match Utc.timestamp_opt(ts, 0) {
                chrono::LocalResult::Single(dt) => dt.to_rfc3339(),
                _ => String::new(),
            }
        } else {
            String::new()
        };

        let name = c.names
            .as_ref()
            .and_then(|names| names.first())
            .map(|n| n.trim_start_matches('/').to_string())
            .unwrap_or_default();

        items.push(Sandbox {
            id: resolve_sandbox_id(c.labels.as_ref(), ""),
            name,
            status: c.state.as_ref().map(|s| s.to_string()).unwrap_or_default(),
            image: c.image.clone().unwrap_or_default(),
            created_at,
            metadata: c.labels.clone().unwrap_or_default(),
        });
    }

    // 返回过滤后的沙箱列表。
    R::ok(ListSandboxesResp {
        items,
    })
}

fn docker_connect_error(error: bollard::errors::Error) -> WebError {
    WebError::BizWithArgs(
        Code::DockerConnectError.into(),
        vec![("error".to_string(), error.to_string())],
    )
}
