#[cfg(any(target_os = "linux", test))]
use std::collections::HashMap;
use std::convert::Infallible;
#[cfg(target_os = "macos")]
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

/// 获取当前系统资源指标的单次快照。
///
/// **用途**：响应普通的 HTTP GET 请求，按需拉取最新的 CPU、内存信息。
/// **参数**：无。
/// **返回值**：成功时返回 JSON 格式的 `Metrics` 数据；失败时返回 500 状态码及错误详情文本。
/// **异常**：不会发生 Panic；底层采集的错误会被捕获并转换为 500 响应。
pub(crate) async fn get_metrics() -> impl IntoResponse {
    match collect_metrics().await {
        Ok(metrics) => Json(metrics).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

/// 建立 SSE（Server-Sent Events）连接，每秒推送一次系统资源指标。
///
/// **用途**：提供实时的系统监控数据流。
/// **参数**：无。
/// **返回值**：返回 `text/event-stream` 响应。
/// **异常**：不会发生 Panic；若单次采集失败，将推送包含 `error` 字段的 JSON，连接不会中断。
pub(crate) async fn watch_metrics() -> impl IntoResponse {
    let (tx, rx) = mpsc::unbounded_channel::<String>();

    // 警告：当前实现为每个客户端启动一个独立的采集轮询任务。
    // 在生产环境中，若存在大量客户端并发连接，将导致严重的性能问题（如耗尽 Blocking 线程池）。
    // 理想方案应采用全局单例采集 + tokio::sync::broadcast 广播。
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        // 确保当系统负载高导致错失 Tick 时，直接顺延，避免短时间内突发大量采集请求
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            ticker.tick().await;

            let payload = match collect_metrics().await {
                Ok(metrics) => serde_json::to_string(&metrics).unwrap_or_else(|_| "{}".to_string()),
                Err(error) => {
                    let err_res = MetricsError { error };
                    serde_json::to_string(&err_res).unwrap_or_else(|_| {
                        r#"{"error":"unknown serialization error"}"#.to_string()
                    })
                }
            };

            // 如果客户端断开连接，tx.send 会失败，此时安全退出后台任务，避免协程泄漏
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

/// 调度系统指标采集任务。
///
/// **用途**：将 CPU 和内存等阻塞型数据采集操作，桥接到 Tokio 的 Blocking 线程池执行，
/// 防止阻塞 Async Worker 线程，导致其他并发请求卡死。
/// **参数**：无。
/// **返回值**：成功时返回 `Metrics` 对象；失败时返回字符串描述的错误原因。
/// **异常**：如果 Blocking 线程崩溃（Panic），会捕获 JoinError 并转为字符串错误返回。
async fn collect_metrics() -> Result<Metrics, String> {
    tokio::task::spawn_blocking(read_metrics_blocking)
        .await
        .map_err(|error| format!("指标采集任务执行崩溃: {error}"))?
}

/// 阻塞式地读取系统的各项底层硬件指标。
///
/// **用途**：整合 CPU、内存及时间戳的采集流程。
/// **参数**：无。
/// **返回值**：成功返回 `Metrics`，失败返回错误原因。
/// **异常**：将底层各个采集模块的错误向上透传。
fn read_metrics_blocking() -> Result<Metrics, String> {
    // 动态获取当前系统的逻辑核心数，作为 CPU 使用率基准，失败则安全回退为 1.0
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

/// 获取当前系统的整体 CPU 使用率。
///
/// **用途**：屏蔽操作系统差异，返回标准化的百分比 (0.0 - 100.0)。
/// **参数**：`cpu_count` (f64) 逻辑核心数，用于在部分平台（如 macOS）上折算总体使用率。
/// **返回值**：0.0 到 100.0 之间的浮点数。
/// **异常**：如果命令执行失败、无权限读取文件或解析失败，返回错误文本。
#[allow(unused_variables)]
fn read_cpu_used_pct(cpu_count: f64) -> Result<f64, String> {
    #[cfg(target_os = "linux")]
    {
        // Linux 必须通过两次时间间隔的 /proc/stat 快照来计算增量 CPU 使用率
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
            .map_err(|error| format!("执行 ps 命令失败: {error}"))?;

        if !output.status.success() {
            return Err(format!("ps 退出码异常: {}", output.status));
        }

        let stdout = String::from_utf8(output.stdout)
            .map_err(|error| format!("ps 输出非 UTF-8 编码: {error}"))?;

        // 采用函数式迭代，安全过滤空行与解析错误行，避免了手动状态维护
        let parsed_values: Vec<f64> = stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .filter_map(|line| line.parse::<f64>().ok())
            .collect();

        if parsed_values.is_empty() {
            return Err("未能在 ps 输出中解析到任何有效的 CPU 使用率数据".to_string());
        }

        let sum: f64 = parsed_values.into_iter().sum();

        // 确保结果不会因为超载等异常情况超出 100% 边界
        Ok((sum / cpu_count).clamp(0.0, 100.0))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = cpu_count;
        Err("当前平台暂不支持 CPU 指标采集".to_string())
    }
}

/// 获取系统内存总量与已使用量。
///
/// **用途**：屏蔽操作系统差异，以 MiB 为单位返回内存数据。
/// **参数**：无。
/// **返回值**：成功返回一个元组 `(总量_MiB, 已使用_MiB)`。
/// **异常**：文件读取失败或命令执行失败时抛出错误文本。
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
            .map_err(|error| format!("vm_stat 输出非 UTF-8 编码: {error}"))?;

        let (page_size, free_pages) = parse_vm_stat_pages(&output)?;
        // 使用 saturating_sub 避免并发波动导致的数值溢出，保证健壮性
        let used_bytes = total_bytes.saturating_sub(free_pages.saturating_mul(page_size));
        Ok((to_mib(total_bytes), to_mib(used_bytes)))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err("当前平台暂不支持内存指标采集".to_string())
    }
}

/// 提取 Linux 系统全局 CPU 统计信息。
///
/// **用途**：从 /proc/stat 读取原始时间片数据。
/// **参数**：无。
/// **返回值**：返回 `CpuSample`。
/// **异常**：无权限读取或解析格式改变时报错。
#[cfg(target_os = "linux")]
fn read_linux_cpu_sample() -> Result<CpuSample, String> {
    let content = fs::read_to_string("/proc/stat")
        .map_err(|error| format!("读取 /proc/stat 失败: {error}"))?;
    parse_linux_cpu_sample(&content)
}

/// 解析 /proc/stat 首行以生成 CPU 采样快照。
///
/// **用途**：将系统的 Jiffies 计数转换为易于比对的 Idle 与 Total。
/// **参数**：`content` (/proc/stat 的全部文本内容)。
/// **返回值**：成功返回 `CpuSample` 实例。
/// **异常**：首行缺失、非 `cpu` 开头或数字解析失败。
#[cfg(any(target_os = "linux", test))]
fn parse_linux_cpu_sample(content: &str) -> Result<CpuSample, String> {
    let first = content
        .lines()
        .next()
        .ok_or_else(|| "/proc/stat 内容为空，缺失首行".to_string())?;

    let fields: Vec<&str> = first.split_whitespace().collect();
    if fields.len() < 5 || fields[0] != "cpu" {
        return Err("不符合预期的 /proc/stat 'cpu' 行格式".to_string());
    }

    // 解析时间片，安全跳过首位字符串
    let mut numbers = Vec::with_capacity(fields.len().saturating_sub(1));
    for raw in fields.iter().skip(1) {
        numbers.push(
            raw.parse::<u64>()
                .map_err(|error| format!("无法将 CPU 字段 '{raw}' 解析为整数: {error}"))?,
        );
    }

    // idle 时间 = idle (下标3) + iowait (下标4)
    let idle = numbers.get(3).copied().unwrap_or(0) + numbers.get(4).copied().unwrap_or(0);
    let total = numbers.into_iter().sum::<u64>();

    Ok(CpuSample { idle, total })
}

/// 根据时间间隔的两次采样计算出 CPU 实际使用率。
///
/// **用途**：计算 delta 并得出 0.0-100.0 的百分比。
/// **参数**：
/// - `first`: 前一次采样的 `CpuSample`。
/// - `second`: 后一次采样的 `CpuSample`。
///
/// **返回值**：0.0 到 100.0 的浮点数。
/// **异常**：不会报错；如遇到负增量或除零，安全回退到 0.0。
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

/// 解析 /proc/meminfo 文件提取核心内存信息。
///
/// **用途**：计算 Linux 的总内存与当前使用内存 (已扣除缓存/Buffer)。
/// **参数**：`content` (/proc/meminfo 的文本内容)。
/// **返回值**：成功返回 `(总量_MiB, 使用量_MiB)`。
/// **异常**：若找不到 `MemTotal` 或任何可计算 `MemAvailable` 的相关字段，将返回错误。
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
        .ok_or_else(|| "/proc/meminfo 缺失基础字段 'MemTotal'".to_string())?;

    // 优先采用现代内核提供的 MemAvailable，若无则手动通过 Free+Buffers+Cached 近似计算
    let available_kib = map
        .get("MemAvailable")
        .copied()
        .or_else(|| {
            let free = map.get("MemFree").copied()?;
            let buffers = map.get("Buffers").copied().unwrap_or(0);
            let cached = map.get("Cached").copied().unwrap_or(0);
            Some(free + buffers + cached)
        })
        .ok_or_else(|| "/proc/meminfo 缺失可用内存相关字段".to_string())?;

    let used_kib = total_kib.saturating_sub(available_kib);

    Ok((total_kib as f64 / 1024.0, used_kib as f64 / 1024.0))
}

/// 从字符串中提取第一组连续的数字。
///
/// **用途**：用于处理带有后缀单位的字符串，如 "102400 kB" -> 102400。
/// **参数**：`text` 包含数字的目标文本。
/// **返回值**：找到有效数字返回 `Some(u64)`，否则返回 `None`。
/// **异常**：无。
fn parse_first_u64(text: &str) -> Option<u64> {
    let digits: String = text
        .chars()
        .skip_while(|char| !char.is_ascii_digit())
        .take_while(|char| char.is_ascii_digit())
        .collect();

    if digits.is_empty() {
        return None;
    }
    digits.parse::<u64>().ok()
}

/// 解析 macOS 下 vm_stat 输出的页数据。
///
/// **用途**：通过提取 Pages free 与 Pages speculative，结合页大小计算剩余内存。
/// **参数**：`content` (vm_stat 命令的原始输出文本)。
/// **返回值**：成功返回 `(单页字节大小, 空闲页数量)`。
/// **异常**：如果输出完全不符合预期，导致缺失 page_size，则报错。
#[cfg(any(target_os = "macos", test))]
fn parse_vm_stat_pages(content: &str) -> Result<(u64, u64), String> {
    let mut lines = content.lines();
    let header = lines
        .next()
        .ok_or_else(|| "vm_stat 命令输出完全为空".to_string())?;

    let page_size = parse_first_u64(header)
        .ok_or_else(|| "vm_stat 首行缺失页面大小 (page size) 信息".to_string())?;

    let mut free_pages = 0_u64;
    let mut speculative_pages = 0_u64;

    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let pages = parse_first_u64(value).unwrap_or(0);
        match key.trim() {
            "Pages free" => free_pages = pages,
            "Pages speculative" => speculative_pages = pages, // speculative 页也算作随时可用的物理内存
            _ => {}
        }
    }

    Ok((page_size, free_pages + speculative_pages))
}

/// 通过 macOS 系统调用命令行获取指定的系统变量配置（如物理内存总大小）。
///
/// **用途**：读取类似 `hw.memsize` 的只读硬件属性。
/// **参数**：`name` (sysctl 查询的具体键名)。
/// **返回值**：成功返回提取的 `u64` 整数值。
/// **异常**：命令执行失败或输出内容无法转为整数时报错。
#[cfg(target_os = "macos")]
fn read_sysctl_u64(name: &str) -> Result<u64, String> {
    let output = StdCommand::new("sysctl")
        .args(["-n", name])
        .output()
        .map_err(|error| format!("执行 sysctl -n {name} 失败: {error}"))?;

    if !output.status.success() {
        return Err(format!("sysctl {name} 退出码异常: {}", output.status));
    }

    let value = String::from_utf8(output.stdout)
        .map_err(|error| format!("sysctl 输出包含非 UTF-8 字符: {error}"))?;

    value
        .trim()
        .parse::<u64>()
        .map_err(|error| format!("无法将 sysctl {name} 的值转换为整数: {error}"))
}

/// 工具函数：将字节数转换为 MiB 大小。
///
/// **用途**：标准化数据响应单位，1 MiB = 1024 * 1024 Bytes。
/// **参数**：`bytes` 输入字节数。
/// **返回值**：对应大小的 `f64`。
#[cfg(target_os = "macos")]
fn to_mib(bytes: u64) -> f64 {
    bytes as f64 / 1024.0 / 1024.0
}

/// 工具函数：获取当前的 Unix 毫秒级时间戳。
///
/// **用途**：在数据包中追加发生时间点，供客户端监控面板时序展示。
/// **参数**：无。
/// **返回值**：返回 u64 格式的毫秒值。
fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0) // 发生时钟回退时降级回 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_linux_cpu_and_calc_used_pct() {
        let first =
            parse_linux_cpu_sample("cpu  100 0 100 800 0 0 0 0 0 0\n").expect("parse first");
        let second =
            parse_linux_cpu_sample("cpu  120 0 120 840 0 0 0 0 0 0\n").expect("parse second");
        let pct = calc_cpu_used_pct(first, second);
        assert!((pct - 50.0).abs() < 0.001);
    }

    #[test]
    fn parse_linux_meminfo_mib_works() {
        let input = "\
MemTotal:       102400 kB
MemFree:         10240 kB
MemAvailable:    30720 kB
Buffers:          1024 kB
Cached:           4096 kB
";
        let (total, used) = parse_linux_meminfo_mib(input).expect("parse meminfo");
        assert!((total - 100.0).abs() < 0.001);
        assert!((used - 70.0).abs() < 0.001);
    }

    #[test]
    fn parse_vm_stat_pages_works() {
        let input = "\
Mach Virtual Memory Statistics: (page size of 4096 bytes)
Pages free:                               100.
Pages active:                             200.
Pages speculative:                         25.
";
        let (page_size, free_pages) = parse_vm_stat_pages(input).expect("parse vm_stat");
        assert_eq!(page_size, 4096);
        assert_eq!(free_pages, 125);
    }
}
