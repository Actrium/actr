use actor_rtc_framework::prelude::*;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use shared_protocols::actor::{ActorId, ActorType, ActorTypeCode};
use shared_protocols::echo::{EchoRequest, EchoResponse};
use std::sync::Arc;
use tokio::runtime::Runtime;

// 简单的测试 Actor
#[derive(Default)]
struct BenchActor {
    counter: Arc<tokio::sync::Mutex<u64>>,
}

// 不实现 ILifecycle，因为已有泛型实现
// BenchActor 自动获得 ILifecycle 实现

#[async_trait::async_trait]
impl MessageHandler<EchoRequest> for BenchActor {
    type Response = EchoResponse;

    async fn handle(
        &self,
        request: EchoRequest,
        _ctx: Arc<Context>,
    ) -> ActorResult<Self::Response> {
        let mut counter = self.counter.lock().await;
        *counter += 1;

        Ok(EchoResponse {
            reply: format!("Echo: {}", request.message),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        })
    }
}

fn create_test_context() -> Arc<Context> {
    let actor_id = ActorId {
        serial_number: 999,
        r#type: Some(ActorType {
            code: ActorTypeCode::Authenticated as i32,
            manufacturer: Some("test".to_string()),
            name: "bench_actor".to_string(),
        }),
    };

    let handle = ActorSystemHandle::placeholder();
    Arc::new(Context::new(actor_id, None, handle))
}

fn bench_message_handling(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    c.benchmark_group("message_handling")
        .bench_function("echo_message", |b| {
            b.iter_custom(|iters| {
                let start = std::time::Instant::now();
                rt.block_on(async {
                    let actor = BenchActor::default();
                    let ctx = create_test_context();

                    for i in 0..iters {
                        let request = EchoRequest {
                            message: format!("test_{}", i),
                            client_id: Some(format!("bench_client_{}", i)),
                        };
                        let _response = actor.handle(request, ctx.clone()).await.unwrap();
                    }
                });
                start.elapsed()
            })
        });
}

fn bench_actor_creation(c: &mut Criterion) {
    c.benchmark_group("actor_creation")
        .bench_function("create_actor", |b| {
            b.iter(|| black_box(BenchActor::default()))
        });
}

fn bench_context_creation(c: &mut Criterion) {
    c.benchmark_group("context_creation")
        .bench_function("create_context", |b| {
            b.iter(|| black_box(create_test_context()))
        });
}

criterion_group!(
    benches,
    bench_message_handling,
    bench_actor_creation,
    bench_context_creation
);
criterion_main!(benches);
