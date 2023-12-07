use std::sync::{Arc, Mutex};

// use swiftlink_dns::DnsResolver;
use swiftlink_infra::fakedns::FakeDns;

pub struct Context {
    // dns_resolver: Arc<DnsResolver>,
    ipv6: bool,
}

pub type SharedContext = Arc<Context>;

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

impl Context {
    pub fn new() -> Context {
        Context {
            // dns_resolver: Arc::new(DnsResolver::system_resolver()),
            ipv6: false,
        }
    }

    pub fn new_shared() -> SharedContext {
        SharedContext::new(Context::new())
    }
}

pub struct AppContext {
    context: SharedContext,
    fakedns: Option<Arc<Mutex<FakeDns>>>,
}

impl AppContext {
    pub fn new() -> Self {
        Self {
            context: Context::new_shared(),
            fakedns: None,
        }
    }

    pub fn set_fakedns(&mut self, fakedns: Arc<Mutex<FakeDns>>) {
        self.fakedns = Some(fakedns);
    }

    pub fn fakedns(&self) -> Option<Arc<Mutex<FakeDns>>> {
        self.fakedns.clone()
    }
}

impl Default for AppContext {
    fn default() -> Self {
        AppContext::new()
    }
}
