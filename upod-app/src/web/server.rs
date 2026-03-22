use crate::web::middleware::fallback;
use crate::web::middleware::i18n::handle_i18n;
use axum::Router;
use axum::middleware::from_fn;
use tokio::signal;
use tracing::{error, info};
use upod_base::web::error::WebError;

pub struct WebServer {
    router: Router,
    addr: String,
    middlewares: Vec<Box<dyn FnOnce(Router) -> Router + Send>>,
}

impl WebServer {
    pub fn new(addr: impl Into<String>) -> Self {
        Self {
            router: Router::new(),
            addr: addr.into(),
            middlewares: Vec::new(),
        }
    }

    pub fn layer_i18n(mut self) -> Self {
        self.middlewares
            .push(Box::new(|r| r.layer(from_fn(handle_i18n))));
        self
    }

    pub fn mount(mut self, router: Router) -> Self {
        self.router = self.router.merge(router);
        self
    }

    pub async fn start(mut self) -> Result<(), WebError> {
        info!("🚀 Starting web server at {}", self.addr);

        self.router = self
            .router
            .method_not_allowed_fallback(fallback::method_not_allowed)
            .fallback(fallback::not_found);

        for m in self.middlewares {
            self.router = m(self.router);
        }

        let listener = tokio::net::TcpListener::bind(&self.addr).await?;

        // 优雅关闭处理
        let server = axum::serve(listener, self.router).with_graceful_shutdown(wait_for_shutdown());
        if let Err(e) = server.await {
            error!("Server error: {}", e);
            return Err(WebError::Sys(format!("Server error: {}", e)));
        }

        info!("🛑 Server stopped");
        Ok(())
    }
}

async fn wait_for_shutdown() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C, starting graceful shutdown");
        },
        _ = terminate => {
            info!("Received terminate signal, starting graceful shutdown");
        },
    }
}
