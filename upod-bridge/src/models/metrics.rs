use serde::Serialize;

/// 系统资源指标响应体。
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Metrics {
    /// 可用 CPU 核数。
    pub(crate) cpu_count: f64,
    /// CPU 使用率（0~100）。
    pub(crate) cpu_used_pct: f64,
    /// 内存总量（MiB）。
    pub(crate) mem_total_mib: f64,
    /// 内存已用（MiB）。
    pub(crate) mem_used_mib: f64,
    /// 采样时间戳（Unix 毫秒）。
    pub(crate) timestamp: u64,
}

/// SSE 流中的错误负载。
#[derive(Debug, Serialize)]
pub(crate) struct MetricsError {
    /// 错误文本。
    pub(crate) error: String,
}

/// Linux `/proc/stat` 采样结果。
#[derive(Debug, Clone, Copy)]
#[cfg(any(target_os = "linux", test))]
pub(crate) struct CpuSample {
    /// 空闲时间片（idle + iowait）。
    pub(crate) idle: u64,
    /// 全量时间片。
    pub(crate) total: u64,
}
