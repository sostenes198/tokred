//! The hook pipeline of a `/v1/messages` request. Fail-open: a hook that errors or panics
//! discards every change and the original bytes are forwarded; a body that is not JSON is
//! forwarded untouched without running any hook.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use bytes::Bytes;
use hyper::HeaderMap;
use serde_json::Value;
use tokred_core::Timestamp;
use tokred_core::proxy::{
    HookNotes, Origin, ProxyHook, RequestCtx, RequestMeta, RequestShape, ResponseSummary,
    classify_origin,
};

/// A request after the hooks ran: what to send first and what to hand to `after`/`fallback`.
#[derive(Debug)]
pub(crate) struct Prepared {
    /// Header metadata.
    pub meta: RequestMeta,
    /// Shape of the original body.
    pub shape: RequestShape,
    /// Who sent it.
    pub origin: Origin,
    /// The body as the hooks left it (`None` when the body is not JSON: no hook ran).
    pub body: Option<Value>,
    /// The hooks changed the body and `send` is its re-serialization.
    pub changed: bool,
    /// What the hooks did.
    pub notes: HookNotes,
    /// Bytes to send upstream first.
    pub send: Bytes,
    /// A hook failed and its changes (and every other hook's) were discarded.
    pub failed: bool,
}

/// Reads the `x-claude-code-*` and `anthropic-beta` metadata from request headers.
pub(crate) fn meta_from(headers: &HeaderMap) -> RequestMeta {
    RequestMeta::from_headers(
        headers
            .iter()
            .filter_map(|(name, value)| Some((name.as_str(), value.to_str().ok()?))),
    )
}

/// Text of a panic payload.
pub(crate) fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "non-text panic payload".to_owned()
    }
}

/// Runs `before` on every hook; the first error or panic stops the pipeline.
fn run_before(hooks: &[Arc<dyn ProxyHook>], ctx: &mut RequestCtx<'_>) -> Result<(), String> {
    for hook in hooks {
        let failure = match catch_unwind(AssertUnwindSafe(|| hook.before(ctx))) {
            Ok(Ok(())) => continue,
            Ok(Err(failure)) => failure.to_string(),
            Err(payload) => format!("{}: panicked: {}", hook.name(), panic_message(&*payload)),
        };
        return Err(failure);
    }
    Ok(())
}

/// Parses the body, classifies the request and runs the hooks.
pub(crate) fn prepare(
    headers: &HeaderMap,
    original: &Bytes,
    hooks: &[Arc<dyn ProxyHook>],
    now: Timestamp,
) -> Prepared {
    let meta = meta_from(headers);
    let parsed = serde_json::from_slice::<Value>(original).ok();
    let shape = parsed.as_ref().map(RequestShape::of).unwrap_or_default();
    let origin = classify_origin(&meta, &shape);
    let Some(body) = parsed else {
        return Prepared {
            meta,
            shape,
            origin,
            body: None,
            changed: false,
            notes: HookNotes::default(),
            send: original.clone(),
            failed: false,
        };
    };
    let mut ctx = RequestCtx {
        meta: &meta,
        shape: &shape,
        origin: &origin,
        now,
        body,
        changed: false,
        notes: HookNotes::default(),
    };
    let outcome = run_before(hooks, &mut ctx);
    let RequestCtx {
        body,
        changed,
        notes,
        ..
    } = ctx;
    let (body, changed, notes, failed) = match outcome {
        Ok(()) => (body, changed, notes, false),
        Err(failure) => {
            tracing::warn!(%failure, "a proxy hook failed; forwarding the original request");
            // The hooks may have changed the body in place: read the original again.
            let original_body = serde_json::from_slice(original).unwrap_or(Value::Null);
            (original_body, false, HookNotes::default(), true)
        }
    };
    let send = if changed {
        serde_json::to_vec(&body).map_or_else(|_| original.clone(), Bytes::from)
    } else {
        original.clone()
    };
    Prepared {
        meta,
        shape,
        origin,
        body: Some(body),
        changed,
        notes,
        send,
        failed,
    }
}

/// Builds the context hooks see after the request (the body is moved in and out).
fn with_ctx(prepared: &mut Prepared, now: Timestamp, call: &mut dyn FnMut(&RequestCtx<'_>)) {
    let Some(body) = prepared.body.take() else {
        return;
    };
    let ctx = RequestCtx {
        meta: &prepared.meta,
        shape: &prepared.shape,
        origin: &prepared.origin,
        now,
        body,
        changed: prepared.changed,
        notes: prepared.notes.clone(),
    };
    call(&ctx);
    prepared.body = Some(ctx.body);
}

/// Calls `call` on every hook, isolating panics (logged, never propagated).
fn each_hook(hooks: &[Arc<dyn ProxyHook>], stage: &str, call: &dyn Fn(&dyn ProxyHook)) {
    for hook in hooks {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| call(hook.as_ref()))) {
            let message = panic_message(&*payload);
            tracing::warn!(hook = hook.name(), stage, %message, "a proxy hook panicked");
        }
    }
}

/// Tells every hook the rewritten request was rejected and the original was sent instead.
pub(crate) fn run_fallback(
    hooks: &[Arc<dyn ProxyHook>],
    prepared: &mut Prepared,
    now: Timestamp,
    status: u16,
) {
    with_ctx(prepared, now, &mut |ctx: &RequestCtx<'_>| {
        each_hook(hooks, "fallback", &|hook| hook.fallback(ctx, status));
    });
}

/// Hands the response summary to every hook (only for requests the hooks saw).
pub(crate) fn run_after(
    hooks: &[Arc<dyn ProxyHook>],
    prepared: &mut Prepared,
    now: Timestamp,
    summary: &ResponseSummary,
) {
    with_ctx(prepared, now, &mut |ctx: &RequestCtx<'_>| {
        each_hook(hooks, "after", &|hook| hook.after(ctx, summary));
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use hyper::header::HeaderValue;
    use serde_json::json;
    use tokred_core::proxy::{HookFailure, rewrite_model, rewrite_ticket};

    use super::*;

    #[derive(Default)]
    struct Spy {
        seen: Mutex<Vec<String>>,
    }

    impl Spy {
        fn push(&self, what: String) {
            self.seen.lock().unwrap().push(what);
        }
    }

    enum Act {
        Nothing,
        Rewrite,
        Mutate,
        Fail,
        Panic,
        PanicString,
        PanicOther,
    }

    struct TestHook {
        act: Act,
        spy: Arc<Spy>,
    }

    impl ProxyHook for TestHook {
        fn name(&self) -> &'static str {
            "test"
        }

        fn before(&self, ctx: &mut RequestCtx<'_>) -> Result<(), HookFailure> {
            self.spy.push(format!("before:{}", ctx.origin.name()));
            match self.act {
                Act::Nothing => Ok(()),
                Act::Rewrite => {
                    let ticket = rewrite_ticket(ctx.origin).unwrap();
                    rewrite_model(&mut ctx.body, &ticket, "claude-haiku-5-5");
                    ctx.changed = true;
                    ctx.notes.model_rewritten = Some("claude-haiku-5-5".to_owned());
                    Ok(())
                }
                Act::Mutate => {
                    ctx.body["model"] = json!("mutated");
                    Err(HookFailure {
                        hook: "test",
                        message: "half done".to_owned(),
                    })
                }
                Act::Fail => Err(HookFailure {
                    hook: "test",
                    message: "nope".to_owned(),
                }),
                Act::Panic => panic!("boom"),
                Act::PanicString => std::panic::panic_any(String::from("owned boom")),
                Act::PanicOther => std::panic::panic_any(7_u8),
            }
        }

        fn after(&self, ctx: &RequestCtx<'_>, response: &ResponseSummary) {
            self.spy.push(format!(
                "after:{}:{}:{}",
                ctx.body["model"], ctx.changed, response.status
            ));
            if matches!(self.act, Act::Panic) {
                panic!("after boom");
            }
        }

        fn fallback(&self, ctx: &RequestCtx<'_>, status: u16) {
            self.spy
                .push(format!("fallback:{}:{status}", ctx.body["model"]));
        }
    }

    fn hooks(acts: Vec<Act>) -> (Vec<Arc<dyn ProxyHook>>, Arc<Spy>) {
        let spy = Arc::new(Spy::default());
        let hooks = acts
            .into_iter()
            .map(|act| {
                Arc::new(TestHook {
                    act,
                    spy: spy.clone(),
                }) as Arc<dyn ProxyHook>
            })
            .collect();
        (hooks, spy)
    }

    fn subagent_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-claude-code-agent-id", HeaderValue::from_static("a1"));
        headers.insert("x-bin", HeaderValue::from_bytes(b"\xff").unwrap());
        headers
    }

    const BODY: &[u8] =
        br#"{ "model" : "claude-sonnet-5-5", "max_tokens": 1.50, "tools": [{"name":"Read"}],
        "messages": [{"role":"user","content":"caf\u00e9"}] }"#;

    #[test]
    fn untouched_requests_forward_the_original_bytes() {
        let (hooks, spy) = hooks(vec![Act::Nothing, Act::Nothing]);
        let original = Bytes::from_static(BODY);
        let prepared = prepare(&subagent_headers(), &original, &hooks, Timestamp(1));
        assert_eq!(prepared.send, original);
        assert!(!prepared.changed && !prepared.failed);
        assert_eq!(prepared.origin.name(), "subagent");
        assert_eq!(prepared.meta.agent_id.as_deref(), Some("a1"));
        assert_eq!(prepared.shape.tool_names, ["Read"]);
        assert_eq!(
            *spy.seen.lock().unwrap(),
            ["before:subagent", "before:subagent"]
        );
    }

    #[test]
    fn a_changed_body_is_reserialized() {
        let (hooks, _spy) = hooks(vec![Act::Rewrite, Act::Nothing]);
        let prepared = prepare(
            &subagent_headers(),
            &Bytes::from_static(BODY),
            &hooks,
            Timestamp(1),
        );
        assert!(prepared.changed);
        let sent: Value = serde_json::from_slice(&prepared.send).unwrap();
        assert_eq!(sent["model"], "claude-haiku-5-5");
        assert_eq!(sent["max_tokens"], 1.5);
        assert_eq!(
            prepared.notes.model_rewritten.as_deref(),
            Some("claude-haiku-5-5")
        );
        assert_eq!(
            prepared.shape.model.as_deref(),
            Some("claude-sonnet-5-5"),
            "the shape describes the original"
        );
    }

    #[test]
    fn any_failure_discards_every_change() {
        for failing in [
            Act::Mutate,
            Act::Fail,
            Act::Panic,
            Act::PanicString,
            Act::PanicOther,
        ] {
            let (hooks, spy) = hooks(vec![Act::Rewrite, failing, Act::Nothing]);
            let original = Bytes::from_static(BODY);
            let prepared = prepare(&subagent_headers(), &original, &hooks, Timestamp(1));
            assert_eq!(prepared.send, original);
            assert!(prepared.failed && !prepared.changed);
            assert_eq!(prepared.notes, HookNotes::default());
            assert_eq!(
                prepared.body.as_ref().unwrap()["model"],
                "claude-sonnet-5-5"
            );
            assert_eq!(spy.seen.lock().unwrap().len(), 2, "the pipeline stops");
        }
    }

    #[test]
    fn a_body_that_is_not_json_runs_no_hook() {
        let (hooks, spy) = hooks(vec![Act::Rewrite]);
        let original = Bytes::from_static(b"{not json");
        let mut prepared = prepare(&HeaderMap::new(), &original, &hooks, Timestamp(1));
        assert_eq!(prepared.send, original);
        assert!(prepared.body.is_none());
        assert_eq!(prepared.origin.name(), "main");
        run_after(
            &hooks,
            &mut prepared,
            Timestamp(2),
            &ResponseSummary::default(),
        );
        run_fallback(&hooks, &mut prepared, Timestamp(2), 400);
        assert!(spy.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn after_and_fallback_see_the_sent_body_and_survive_panics() {
        let (rewriting, spy) = hooks(vec![Act::Rewrite, Act::Nothing]);
        let mut prepared = prepare(
            &subagent_headers(),
            &Bytes::from_static(BODY),
            &rewriting,
            Timestamp(1),
        );
        run_fallback(&rewriting, &mut prepared, Timestamp(2), 400);
        let summary = ResponseSummary {
            status: 200,
            ..ResponseSummary::default()
        };
        run_after(&rewriting, &mut prepared, Timestamp(3), &summary);
        assert!(prepared.body.is_some(), "the body is put back");
        let seen = spy.seen.lock().unwrap().clone();
        assert_eq!(
            seen[2..],
            [
                "fallback:\"claude-haiku-5-5\":400",
                "fallback:\"claude-haiku-5-5\":400",
                "after:\"claude-haiku-5-5\":true:200",
                "after:\"claude-haiku-5-5\":true:200",
            ]
        );

        let (panicky, spy) = hooks(vec![Act::Panic, Act::Nothing]);
        let mut prepared = prepare(
            &HeaderMap::new(),
            &Bytes::from_static(BODY),
            &[],
            Timestamp(1),
        );
        run_after(&panicky, &mut prepared, Timestamp(3), &summary);
        assert_eq!(
            spy.seen.lock().unwrap().len(),
            2,
            "the next hook still runs"
        );
    }

    #[test]
    fn panic_messages() {
        assert_eq!(panic_message(&"static"), "static");
        assert_eq!(panic_message(&String::from("owned")), "owned");
        assert_eq!(panic_message(&3_i32), "non-text panic payload");
    }
}
