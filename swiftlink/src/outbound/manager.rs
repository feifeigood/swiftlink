use std::collections::HashMap;

use crate::config::Proxy;

use super::AnyOutboundHandle;

pub struct OutboundManager {
    outbounds: HashMap<String, AnyOutboundHandle>,
}

impl OutboundManager {
    fn load_outbound(proxies: &Vec<Proxy>, outbounds: &mut HashMap<String, AnyOutboundHandle>) -> anyhow::Result<()> {
        for proxy in proxies.iter() {
            let name = proxy.name.clone();

            // let handle = match proxy.protocol.as_str() {
            //     "trojan" => {}
            // };
        }

        Ok(())
    }

    pub fn new(proxies: &Vec<Proxy>) -> anyhow::Result<OutboundManager> {
        let mut outbounds: HashMap<String, AnyOutboundHandle> = HashMap::new();

        Self::load_outbound(proxies, &mut outbounds)?;

        Ok(OutboundManager { outbounds })
    }
}
