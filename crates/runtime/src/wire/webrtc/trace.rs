use actr_protocol::SignalingEnvelope;
use tracing::Span;

use opentelemetry::{
    Context,
    propagation::{Extractor, Injector},
    trace::TraceContextExt,
};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Inject the current span into a signaling envelope (no-op if span is invalid).
pub(crate) fn inject_current_span(envelope: &mut SignalingEnvelope) {
    inject_span_context(&Span::current(), envelope);
}

/// Set the given span's parent from the envelope context (or current Context if invalid).
pub(crate) fn set_parent_from_envelope(span: &Span, envelope: &SignalingEnvelope) {
    let context = extract_trace_context(envelope);
    span.set_parent(context);
}

fn inject_span_context(span: &Span, envelope: &mut SignalingEnvelope) {
    let mut injector = EnvelopeInjector(envelope);
    let context = span.context();
    let span_ref = context.span();
    let span_context = span_ref.span_context();
    if !span_context.is_valid() {
        return;
    }

    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&context, &mut injector)
    });
}

pub(crate) fn extract_trace_context(envelope: &SignalingEnvelope) -> Context {
    let context = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&EnvelopeExtractor(envelope))
    });
    let span_ref = context.span();
    let span_context = span_ref.span_context();
    if span_context.is_valid() {
        context
    } else {
        Context::current()
    }
}

struct EnvelopeExtractor<'a>(&'a SignalingEnvelope);

impl<'a> Extractor for EnvelopeExtractor<'a> {
    fn get(&self, key: &str) -> Option<&str> {
        match key {
            "traceparent" => self.0.traceparent.as_deref(),
            "tracestate" => self.0.tracestate.as_deref(),
            _ => None,
        }
    }

    fn keys(&self) -> Vec<&str> {
        vec!["traceparent", "tracestate"]
    }
}

struct EnvelopeInjector<'a>(&'a mut SignalingEnvelope);

impl<'a> Injector for EnvelopeInjector<'a> {
    fn set(&mut self, key: &str, value: String) {
        match key {
            "traceparent" => self.0.traceparent = Some(value),
            "tracestate" => self.0.tracestate = Some(value),
            _ => {}
        }
    }
}
