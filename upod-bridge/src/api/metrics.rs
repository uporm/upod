#[cfg(any(target_os = "linux", test))]
use std::collections::HashMap;
use std::convert::Infallible;
use std::process::Command as StdCommand;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[cfg(target_os = "linux")]
use std::{fs, thread};

use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::UnboundedReceiverStream;

#[cfg(any(target_os = "linux", test))]
use crate::models::metrics::CpuSample;
use crate::models::metrics::{Metrics, MetricsError};

/// 返回当前系统指标快照。
/// 参数：无。
/// 返回：成功时返回 JSON 指标；失败时返回 500 和错误文本。
/// 异常：无；内部错误通过 HTTP 状态返回。
pub(crate) async fn get_metrics() -> impl IntoResponse {
    match collect_metrics().await {
        Ok(metrics) => Json(metrics).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

/// 以 SSE 方式每秒推送一次系统指标。
/// 参数：无。
/// 返回：`text/event-stream` 响应，事件数据为 JSON 字符串。
/// 异常：无；采集失败时输出 `{"error":"..."}` 事件。
pub(crate) async fn watch_metrics() -> impl IntoResponse {
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let payload = match collect_metrics().await {
                Ok(metrics) => serde_json::to_string(&metrics).unwrap_or_else(|_| "{}".to_string()),
                Err(error) => serde_json::to_string(&MetricsError { error })
                    .unwrap_or_else(|_| "{\"error\":\"unknown\"}".to_string()),
            };
            if tx.send(payload).is_err() {
                break;
            }
        }
    });

    let stream = UnboundedReceiverStream::new(rx)
        .map(|payload| Ok::<Event, Infallible>(Event::default().data(payload)));
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))
        .into_response()
}

/// 在阻塞线程中采集系统指标，避免阻塞 async 运行时。
/// 参数：无。
/// 返回：成功时返回 `Metrics`，失败时返回错误文本。
/// 异常：`spawn_blocking` 任务崩溃时返回 join 错误。
async fn collect_metrics() -> Result<Metrics, String> {
    tokio::task::spawn_blocking(read_metrics_blocking)
        .await
        .map_err(|error| format!("指标采集任务失败: {error}"))?
}

/// 采集 CPU、内存和时间戳。
/// 参数：无。
/// 返回：成功时返回完整指标快照。
/// 异常：底层采样失败时返回错误文本。
fn read_metrics_blocking() -> Result<Metrics, String> {
    let cpu_count = std::thread::available_parallelism()
        .map(|value| value.get() as f64)
        .unwrap_or(1.0);
    let cpu_used_pct = read_cpu_used_pct(cpu_count)?;
    let (mem_total_mib, mem_used_mib) = read_mem_mib()?;
    Ok(Metrics {
        cpu_count,
        cpu_used_pct,
        mem_total_mib,
        mem_used_mib,
        timestamp: now_millis(),
    })
}

/// 获取 CPU 使用率。
/// 参数：`cpu_count` 为核数，用于按平台换算百分比。
/// 返回：0~100 的 CPU 使用率。
/// 异常：采样失败时返回错误文本。
fn read_cpu_used_pct(cpu_count: f64) -> Result<f64, String> {
    #[cfg(target_os = "linux")]
    {
        let first = read_linux_cpu_sample()?;
        thread::sleep(Duration::from_secs(1));
        let second = read_linux_cpu_sample()?;
        Ok(calc_cpu_used_pct(first, second))
    }
    #[cfg(target_os = "macos")]
    {
        let output = StdCommand::new("ps")
            .args(["-A", "-o", "%cpu="])
            .output()
            .map_err(|error| format!("执行 ps 失败: {error}"))?;
        if !output.status.success() {
            return Err(format!("ps 退出码异常: {}", output.status));
        }
        let stdout = String::from_utf8(output.stdout)
            .map_err(|error| format!("ps 输出编码错误: {error}"))?;
        let mut sum = 0.0_f64;
        let mut count = 0_u64;
        for line in stdout.lines() {
            let value = line.trim();
            if value.is_empty() {
                continue;
            }
            if let Ok(parsed) = value.parse::<f64>() {
                sum += parsed;
                count += 1;
            }
        }
        if count == 0 {
            return Err("未解析到 CPU 使用率数据".to_string());
        }
        Ok((sum / cpu_count).clamp(0.0, 100.0))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = cpu_count;
        Err("当前平台不支持 CPU 指标采集".to_string())
    }
}

/// 获取内存总量与已用量（MiB）。
/// 参数：无。
/// 返回：`(total_mib, used_mib)`。
/// 异常：采样失败时返回错误文本。
fn read_mem_mib() -> Result<(f64, f64), String> {
    #[cfg(target_os = "linux")]
    {
        let content = fs::read_to_string("/proc/meminfo")
            .map_err(|error| format!("读取 /proc/meminfo 失败: {error}"))?;
        parse_linux_meminfo_mib(&content)
    }
    #[cfg(target_os = "macos")]
    {
        let total_bytes = read_sysctl_u64("hw.memsize")?;
        let vm_stat = StdCommand::new("vm_stat")
            .output()
            .map_err(|error| format!("执行 vm_stat 失败: {error}"))?;
        if !vm_stat.status.success() {
            return Err(format!("vm_stat 退出码异常: {}", vm_stat.status));
        }
        let output = String::from_utf8(vm_stat.stdout)
            .map_err(|error| format!("vm_stat 输出编码错误: {error}"))?;
        let (page_size, free_pages) = parse_vm_stat_pages(&output)?;
        let used_bytes = total_bytes.saturating_sub(free_pages.saturating_mul(page_size));
        Ok((to_mib(total_bytes), to_mib(used_bytes)))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err("当前平台不支持内存指标采集".to_string())
    }
}

/// 读取 Linux CPU 采样。
/// 参数：无。
/// 返回：`CpuSample`。
/// 异常：读取或解析 `/proc/stat` 失败时返回错误文本。
#[cfg(target_os = "linux")]
fn read_linux_cpu_sample() -> Result<CpuSample, String> {
    let content = fs::read_to_string("/proc/stat")
        .map_err(|error| format!("读取 /proc/stat 失败: {error}"))?;
    parse_linux_cpu_sample(&content)
}

/// 解析 Linux `/proc/stat` 的首行总 CPU 数据。
/// 参数：`content` 为 `/proc/stat` 文本。
/// 返回：`CpuSample`。
/// 异常：缺少字段或数字解析失败时返回错误文本。
#[cfg(any(target_os = "linux", test))]
fn parse_linux_cpu_sample(content: &str) -> Result<CpuSample, String> {
    let first = content
        .lines()
        .next()
        .ok_or_else(|| "缺少 /proc/stat 首行".to_string())?;
    let fields = first.split_whitespace().collect::<Vec<_>>();
    if fields.len() < 5 || fields[0] != "cpu" {
        return Err("无效的 /proc/stat CPU 行".to_string());
    }
    let mut numbers = Vec::with_capacity(fields.len().saturating_sub(1));
    for raw in fields.iter().skip(1) {
        numbers.push(
            raw.parse::<u64>()
                .map_err(|error| format!("解析 CPU 字段失败: {error}"))?,
        );
    }
    let idle = numbers.get(3).copied().unwrap_or(0) + numbers.get(4).copied().unwrap_or(0);
    let total = numbers.into_iter().sum::<u64>();
    Ok(CpuSample { idle, total })
}

/// 根据两次采样计算 CPU 使用率。
/// 参数：`first` 为第一次采样，`second` 为第二次采样。
/// 返回：0~100 的 CPU 使用率。
/// 异常：无；异常采样差值会回退为 0。
#[cfg(any(target_os = "linux", test))]
fn calc_cpu_used_pct(first: CpuSample, second: CpuSample) -> f64 {
    let total_delta = second.total.saturating_sub(first.total);
    if total_delta == 0 {
        return 0.0;
    }
    let idle_delta = second.idle.saturating_sub(first.idle);
    let used_delta = total_delta.saturating_sub(idle_delta);
    (used_delta as f64 * 100.0 / total_delta as f64).clamp(0.0, 100.0)
}

/// 解析 Linux `/proc/meminfo` 并转换为 MiB。
/// 参数：`content` 为 `/proc/meminfo` 文本。
/// 返回：`(total_mib, used_mib)`。
/// 异常：缺少关键字段或数字解析失败时返回错误文本。
#[cfg(any(target_os = "linux", test))]
fn parse_linux_meminfo_mib(content: &str) -> Result<(f64, f64), String> {
    let mut map = HashMap::<String, u64>::new();
    for line in content.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        if let Some(value) = parse_first_u64(rest) {
            map.insert(key.trim().to_string(), value);
        }
    }

    let total_kib = map
        .get("MemTotal")
        .copied()
        .ok_or_else(|| "meminfo 缺少 MemTotal".to_string())?;
    let available_kib = map
        .get("MemAvailable")
        .copied()
        .or_else(|| {
            let free = map.get("MemFree").copied()?;
            let buffers = map.get("Buffers").copied().unwrap_or(0);
            let cached = map.get("Cached").copied().unwrap_or(0);
            Some(free + buffers + cached)
        })
        .ok_or_else(|| "meminfo 缺少可用内存字段".to_string())?;
    let used_kib = total_kib.saturating_sub(available_kib);
    Ok((total_kib as f64 / 1024.0, used_kib as f64 / 1024.0))
}

/// 从任意文本中提取第一个无符号整数。
/// 参数：`text` 为待解析文本。
/// 返回：存在数字时返回 `Some(u64)`，否则返回 `None`。
/// 异常：无。
fn parse_first_u64(text: &str) -> Option<u64> {
    let digits = text
        .chars()
        .skip_while(|char| !char.is_ascii_digit())
        .take_while(|char| char.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u64>().ok()
}

/// 解析 macOS `vm_stat` 输出，提取页大小和空闲页数。
/// 参数：`content` 为 `vm_stat` 文本。
/// 返回：`(page_size_bytes, free_pages)`。
/// 异常：关键字段缺失或解析失败时返回错误文本。
fn parse_vm_stat_pages(content: &str) -> Result<(u64, u64), String> {
    let mut lines = content.lines();
    let header = lines.next().ok_or_else(|| "vm_stat 输出为空".to_string())?;
    let page_size = parse_first_u64(header).ok_or_else(|| "vm_stat 缺少页面大小".to_string())?;

    let mut free_pages = 0_u64;
    let mut speculative_pages = 0_u64;
    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let pages = parse_first_u64(value).unwrap_or(0);
        match key.trim() {
            "Pages free" => free_pages = pages,
            "Pages speculative" => speculative_pages = pages,
            _ => {}
        }
    }

    Ok((page_size, free_pages + speculative_pages))
}

/// 读取 macOS `sysctl` 数值配置。
/// 参数：`name` 为 sysctl 键名。
/// 返回：解析出的 `u64` 值。
/// 异常：命令执行失败、退出码异常或输出解析失败时返回错误文本。
#[cfg(target_os = "macos")]
fn read_sysctl_u64(name: &str) -> Result<u64, String> {
    let output = StdCommand::new("sysctl")
        .args(["-n", name])
        .output()
        .map_err(|error| format!("执行 sysctl 失败: {error}"))?;
    if !output.status.success() {
        return Err(format!("sysctl {name} 退出码异常: {}", output.status));
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|error| format!("sysctl 输出编码错误: {error}"))?;
    value
        .trim()
        .parse::<u64>()
        .map_err(|error| format!("解析 sysctl {name} 失败: {error}"))
}

/// 将字节数转换为 MiB。
/// 参数：`bytes` 为字节数。
/// 返回：MiB 浮点值。
/// 异常：无。
fn to_mib(bytes: u64) -> f64 {
    bytes as f64 / 1024.0 / 1024.0
}

/// 生成当前 Unix 毫秒时间戳。
/// 参数：无。
/// 返回：毫秒时间戳，系统时钟异常时回退为 0。
/// 异常：无。
fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证 Linux CPU 行解析与使用率计算。
    #[test]
    fn parse_linux_cpu_and_calc_used_pct() {
        println!("开始测试: parse_linux_cpu_and_calc_used_pct");
        let first =
            parse_linux_cpu_sample("cpu  100 0 100 800 0 0 0 0 0 0\n").expect("parse first");
        let second =
            parse_linux_cpu_sample("cpu  120 0 120 840 0 0 0 0 0 0\n").expect("parse second");
        let pct = calc_cpu_used_pct(first, second);
        println!("计算得到 CPU 使用率: {pct}");
        assert!((pct - 50.0).abs() < 0.001);
        println!("测试通过: parse_linux_cpu_and_calc_used_pct");
    }

    /// 验证 Linux 内存信息解析。
    #[test]
    fn parse_linux_meminfo_mib_works() {
        println!("开始测试: parse_linux_meminfo_mib_works");
        let input = "\
MemTotal:       102400 kB
MemFree:         10240 kB
MemAvailable:    30720 kB
Buffers:          1024 kB
Cached:           4096 kB
";
        let (total, used) = parse_linux_meminfo_mib(input).expect("parse meminfo");
        println!("解析结果 total={total}MiB, used={used}MiB");
        assert!((total - 100.0).abs() < 0.001);
        assert!((used - 70.0).abs() < 0.001);
        println!("测试通过: parse_linux_meminfo_mib_works");
    }

    /// 验证 macOS vm_stat 空闲页解析。
    #[test]
    fn parse_vm_stat_pages_works() {
        println!("开始测试: parse_vm_stat_pages_works");
        let input = "\
Mach Virtual Memory Statistics: (page size of 4096 bytes)
Pages free:                               100.
Pages active:                             200.
Pages speculative:                         25.
";
        let (page_size, free_pages) = parse_vm_stat_pages(input).expect("parse vm_stat");
        println!("解析结果 page_size={page_size}, free_pages={free_pages}");
        assert_eq!(page_size, 4096);
        assert_eq!(free_pages, 125);
        println!("测试通过: parse_vm_stat_pages_works");
    }
}
