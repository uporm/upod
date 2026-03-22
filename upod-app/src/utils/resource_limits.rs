pub(crate) fn parse_memory(mem: &str) -> Option<i64> {
    let mem = mem.trim();
    if mem.is_empty() {
        None
    } else {
        parse_memory_with_suffix(mem, 2)
            .or_else(|| parse_memory_with_suffix(mem, 1))
            .or_else(|| mem.parse::<i64>().ok())
    }
}

fn parse_memory_with_suffix(mem: &str, suffix_len: usize) -> Option<i64> {
    if mem.len() <= suffix_len {
        None
    } else {
        let (num, unit) = mem.split_at(mem.len() - suffix_len);
        let value = num.parse::<i64>().ok()?;
        let multiplier = match unit {
            "Mi" => 1024 * 1024,
            "Gi" => 1024 * 1024 * 1024,
            "M" => 1000 * 1000,
            "G" => 1000 * 1000 * 1000,
            _ => return None,
        };

        Some(value * multiplier)
    }
}

pub(crate) fn parse_cpu(cpu: &str) -> Option<i64> {
    let cpu = cpu.trim();
    if cpu.ends_with('m') {
        cpu.trim_end_matches('m')
            .parse::<f64>()
            .ok()
            .map(|v| (v * 1_000_000.0) as i64)
    } else {
        cpu.parse::<f64>()
            .ok()
            .map(|v| (v * 1_000_000_000.0) as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_cpu, parse_memory};

    #[test]
    fn test_parse_memory() {
        assert_eq!(parse_memory("512Mi"), Some(512 * 1024 * 1024));
        assert_eq!(parse_memory("1Gi"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_memory("100M"), Some(100 * 1000 * 1000));
        assert_eq!(parse_memory(""), None);
    }

    #[test]
    fn test_parse_cpu() {
        assert_eq!(parse_cpu("500m"), Some(500_000_000));
        assert_eq!(parse_cpu("1"), Some(1_000_000_000));
        assert_eq!(parse_cpu("0.5"), Some(500_000_000));
    }
}
