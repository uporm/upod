use bollard::service::{ContainerCreateBody, HostConfig, PortBinding};
fn main() {
    let _: Option<std::collections::HashMap<String, Option<Vec<PortBinding>>>> = HostConfig::default().port_bindings;
    let _: Option<std::collections::HashMap<String, std::collections::HashMap<(), ()>>> = ContainerCreateBody::default().exposed_ports;
}
