use axum::http::Request;
use tower_http::{
    classify::{ServerErrorsAsFailures, SharedClassifier},
    trace::{MakeSpan, TraceLayer},
};
use tracing::{Span, info_span};

#[cfg(feature = "opentelemetry")]
use opentelemetry::{Context, propagation::Extractor, trace::TraceContextExt};
#[cfg(feature = "opentelemetry")]
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub type HttpTraceLayer = TraceLayer<SharedClassifier<ServerErrorsAsFailures>, HttpMakeSpan>;

pub fn http_trace_layer() -> HttpTraceLayer {
    TraceLayer::new_for_http().make_span_with(HttpMakeSpan)
}

#[derive(Clone, Debug, Default)]
pub struct HttpMakeSpan;

impl<B> MakeSpan<B> for HttpMakeSpan {
    fn make_span(&mut self, request: &Request<B>) -> Span {
        let span = info_span!(
            "http.request",
            method = %request.method(),
            uri = %request.uri(),
            version = ?request.version()
        );

        #[cfg(feature = "opentelemetry")]
        if let Some(context) = extract_remote_context(request.headers()) {
            let _ = span.set_parent(context);
        }

        span
    }
}

#[cfg(feature = "opentelemetry")]
fn extract_remote_context(headers: &axum::http::HeaderMap) -> Option<Context> {
    struct HeaderExtractor<'a>(&'a axum::http::HeaderMap);

    impl<'a> Extractor for HeaderExtractor<'a> {
        fn get(&self, key: &str) -> Option<&str> {
            self.0.get(key).and_then(|value| value.to_str().ok())
        }

        fn keys(&self) -> Vec<&str> {
            self.0.keys().map(|name| name.as_str()).collect()
        }
    }

    let context = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(headers))
    });
    let span_ref = context.span();
    let span_context = span_ref.span_context();
    if span_context.is_valid() {
        Some(context)
    } else {
        None
    }
}
