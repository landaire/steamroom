//! `tracing_subscriber::Layer` that intercepts events emitted inside a
//! `job_id`-tagged span and republishes them as `Event::Log` for that
//! job. Off-span events have `job_id: None` and only land in the daemon
//! log file (handled by the wrapping fmt layer).

use tokio::sync::broadcast::Sender;
use tracing::{Event as TracingEvent, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use crate::daemon::proto::{Event, JobId, LogLevel};

/// Span field name carrying the job id. The worker sets this when
/// entering each job's span: `tracing::info_span!("job", job_id = %job.0)`.
pub const JOB_ID_FIELD: &str = "job_id";

pub struct JobScopedLogLayer {
    pub events: Sender<Event>,
}

impl JobScopedLogLayer {
    pub fn new(events: Sender<Event>) -> Self { Self { events } }
}

impl<S> Layer<S> for JobScopedLogLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &TracingEvent<'_>, ctx: Context<'_, S>) {
        let job_id = find_job_id_in_scope(event, &ctx);
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let message = visitor.message.unwrap_or_default();

        let level: LogLevel = (*event.metadata().level()).into();
        let target = event.metadata().target().to_string();

        let _ = self.events.send(Event::Log {
            job_id: job_id.map(JobId),
            level,
            target,
            message,
        });
    }
}

fn find_job_id_in_scope<S>(event: &TracingEvent<'_>, ctx: &Context<'_, S>) -> Option<u64>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    let scope = ctx.event_scope(event)?;
    for span in scope.from_root() {
        let ext = span.extensions();
        if let Some(id) = ext.get::<JobIdAttachment>() {
            return Some(id.0);
        }
    }
    None
}

/// Attached to a span by `on_new_span` when the span's recorded fields
/// include `job_id`. Pure data; no Arc.
struct JobIdAttachment(u64);

pub struct JobIdAttachmentInstaller;

impl<S> Layer<S> for JobIdAttachmentInstaller
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &tracing::span::Attributes<'_>, id: &tracing::span::Id, ctx: Context<'_, S>) {
        let mut v = JobIdFieldVisitor::default();
        attrs.record(&mut v);
        if let Some(jid) = v.job_id
            && let Some(span) = ctx.span(id)
        {
            span.extensions_mut().insert(JobIdAttachment(jid));
        }
    }
}

#[derive(Default)]
struct JobIdFieldVisitor { job_id: Option<u64> }

impl tracing::field::Visit for JobIdFieldVisitor {
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        if field.name() == JOB_ID_FIELD { self.job_id = Some(value); }
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        if field.name() == JOB_ID_FIELD && value >= 0 {
            self.job_id = Some(value as u64);
        }
    }
}

#[derive(Default)]
struct MessageVisitor { message: Option<String> }

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}").trim_matches('"').to_string());
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" { self.message = Some(value.to_string()); }
    }
}
