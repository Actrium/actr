use actr_protocol::SignalingEnvelope;
use opentelemetry::{
    Context, propagation::Extractor, propagation::Injector, trace::TraceContextExt,
};

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

/// Extract trace context from SignalingEnvelope.
/// If no valid context is present, returns the current thread Context as a new root.
pub fn extract_trace_context(envelope: &SignalingEnvelope) -> Context {
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

/// Inject the given OpenTelemetry context into SignalingEnvelope.
pub fn inject_trace_context(context: &Context, envelope: &mut SignalingEnvelope) {
    let mut injector = EnvelopeInjector(envelope);
    let span_ref = context.span();
    let span_context = span_ref.span_context();
    if !span_context.is_valid() {
        return;
    }

    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(context, &mut injector)
    });
}
