use std::{thread, time::Duration};

use kutrace_client::{Span, annotation_kind, dropped_events, legacy_event, legacy_marker};

fn emit(event: u16, arg: u64, retval: i32, rpc: u32, label: &str) {
    assert!(
        legacy_marker(event, arg, retval, rpc, label),
        "collector did not accept marker {event:#x}"
    );
    thread::sleep(Duration::from_millis(2));
}

fn main() {
    {
        let span = Span::enter("agent.reason");
        assert!(span.annotate(annotation_kind::QUERY, 11, "agent.query.events"));
        assert!(span.annotate(
            annotation_kind::OBSERVATION,
            22,
            "agent.observation.latency"
        ));
        assert!(span.annotate(annotation_kind::DECISION, 33, "agent.decision.retry"));
        assert!(span.annotate(annotation_kind::RESULT, 44, "agent.result.success"));
    }
    emit(legacy_event::RPC_REQUEST, 77, 0, 77, "agent.rpc.read");
    emit(legacy_event::RESOURCE, 9, 0, 77, "agent.resource");
    emit(legacy_event::ENQUEUE, 3, 0, 77, "agent.queue");
    emit(legacy_event::DEQUEUE, 3, 0, 77, "agent.queue");
    emit(legacy_event::RPC_REQUEST, 0, 0, 0, "agent.rpc.read");
    emit(legacy_event::MARK_A, 123, -5, 0, "agent.mark");
    assert_eq!(dropped_events(), 0);
    println!("{{\"markers\":6,\"annotations\":4,\"client_events_dropped\":0}}");
}
