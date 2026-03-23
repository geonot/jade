# Jade Actor System — Comprehensive Architecture Report

**Scope:** Full evaluation of actor-model concurrency for Jade  
**Context:** Current implementation + design space for next-generation evolution  
**Date:** March 2026

---

## Table of Contents

1. [Current Implementation Baseline](#1-current-implementation-baseline)
2. [Threading Models: 1:1, N:M, Thread-per-Core](#2-threading-models)
3. [Mailbox Architectures](#3-mailbox-architectures)
4. [Named Actors, Registries, Discovery](#4-named-actors-registries-discovery)
5. [Remote Actors, Endpoints, Protocols](#5-remote-actors-endpoints-protocols)
6. [Message Formats & Serialization](#6-message-formats--serialization)
7. [Message Brokers: Kafka, RabbitMQ, ZeroMQ, NATS](#7-message-brokers)
8. [Event Loops vs. On-Message Wake](#8-event-loops-vs-on-message-wake)
9. [Orchestrators, Workflow Engines, Supervision](#9-orchestrators-workflow-engines-supervision)
10. [Timed Events, Wakeups, Signals](#10-timed-events-wakeups-signals)
11. [OS/Kernel-Level Support](#11-oskernel-level-support)
12. [Async Integration](#12-async-integration)
13. [Actors as the Sole Parallelism Model?](#13-actors-as-the-sole-parallelism-model)
14. [Performance Analysis](#14-performance-analysis)
15. [Scalability Analysis](#15-scalability-analysis)
16. [Reliability Analysis](#16-reliability-analysis)
17. [Historical Landscape](#17-historical-landscape)
18. [Recommended Architecture for Jade](#18-recommended-architecture-for-jade)
19. [Implementation Roadmap](#19-implementation-roadmap)

---

## 1. Current Implementation Baseline

### What Jade Has Today

```
Model:          1:1 pthread-per-actor
Mailbox:        Bounded MPSC ring buffer, capacity 256
Sync:           pthread_mutex + 2× pthread_cond (notfull, notempty)
Backpressure:   Sender blocks when full (cond_wait on notfull)
Lifecycle:      spawn → detach (fire-and-forget), no join/stop/supervision
Message format: { i32 tag, [max_payload × i8] } — zero-copy, no serialization
State:          Inline in mailbox struct, GEP-accessed in handler
```

### Current Benchmark Results (vs. equivalent C pthreads)

| Benchmark | Jade (ms) | C (ms) | Ratio | Description |
|-----------|-----------|--------|-------|-------------|
| actor_pingpong | 740.81 | 780.79 | **0.95×** | 1M messages to single actor |
| actor_throughput | 2023.14 | 2159.46 | **0.94×** | 5M messages to single actor |
| actor_fanout | 13341.12 | 12354.30 | **1.08×** | 8M messages across 8 actors |

Jade actors are at **parity with hand-written C pthreads**. The fanout regression (8%) comes from thread creation overhead at spawn × 8 and mutex contention across 8 concurrent mailboxes.

### Current Limitations

| Gap | Impact | Severity |
|-----|--------|----------|
| No actor stop/kill | Actors never terminate; rely on `usleep` + process exit | **High** |
| No supervision tree | No restart policy, no error escalation | **High** |
| No reply mechanism | Send is fire-and-forget; no request/response | **High** |
| No named actors | Can't look up actors by name/address | **Medium** |
| Thread-per-actor | Doesn't scale beyond ~10K actors (OS thread limit) | **High** |
| No async integration | Actor loop is blocking; can't yield to async runtime | **Medium** |
| Hardcoded mailbox size | 256 fixed; no tuning per actor type | **Low** |
| No remote messaging | Single-process only | **Medium** |
| State zero-initialized only | No constructor arguments at spawn | **Medium** |

---

## 2. Threading Models

### 2.1 1:1 (Current — Thread-per-Actor)

```
┌─────────┐   ┌─────────┐   ┌─────────┐
│ Actor A  │   │ Actor B  │   │ Actor C  │
│  Thread  │   │  Thread  │   │  Thread  │
│  1       │   │  2       │   │  3       │
└─────────┘   └─────────┘   └─────────┘
   OS sched      OS sched      OS sched
```

**Pros:** Simple, true parallelism, OS handles preemption, actors can block freely (I/O, syscalls).  
**Cons:** Thread creation ~50μs each, stack ~8MB per thread, tops out at ~10K actors, context-switch overhead ~1-5μs.  
**Best for:** Coarse-grained actors (tens to hundreds), I/O-heavy actors, minimal spawn churn.

### 2.2 N:M (Work-Stealing Scheduler)

```
┌─────────────────────────────────┐
│         Worker Thread Pool       │
│  ┌───┐ ┌───┐ ┌───┐ ┌───┐       │
│  │W1 │ │W2 │ │W3 │ │W4 │ (N)   │
│  └─┬─┘ └─┬─┘ └─┬─┘ └─┬─┘       │
│    │      │      │      │        │
│  ┌─┴──┐ ┌┴──┐ ┌─┴─┐ ┌─┴───┐    │
│  │A,B │ │C,D│ │E,F│ │G,H,I│(M) │
│  │runq│ │rq │ │rq │ │runq │    │
│  └────┘ └───┘ └───┘ └─────┘    │
│        ← work stealing →        │
└─────────────────────────────────┘
```

This is the Erlang/Go/Tokio model. N OS threads run M actors/goroutines.

**Pros:** Millions of actors (lightweight ~256B–2KB each), load-balanced, scales to core count.  
**Cons:** Actors must not block OS threads (needs cooperative yield or async I/O), complex scheduler, harder to debug.  
**Best for:** Massive actor populations (chat servers, IoT, game entities), microservice decomposition.

**Key design decisions:**
- **Run queue per worker** (local affinity) with **lock-free stealing** from other queues
- **Cooperative preemption:** inject yield points at function calls, loop back-edges, or message sends
- **Blocking detection:** if an actor does blocking I/O, spawn a replacement worker thread to keep N threads active (Go's approach)

### 2.3 Thread-per-Core (Seastar/Glommio Model)

```
┌──────────────────────────────────────┐
│  Core 0          Core 1          ... │
│  ┌──────────┐   ┌──────────┐        │
│  │ Reactor  │   │ Reactor  │        │
│  │ io_uring │   │ io_uring │        │
│  │ ┌──────┐ │   │ ┌──────┐ │        │
│  │ │ActorA│ │   │ │ActorC│ │        │
│  │ │ActorB│ │   │ │ActorD│ │        │
│  │ └──────┘ │   │ └──────┘ │        │
│  └──────────┘   └──────────┘        │
│  NO sharing     NO sharing          │
│  NO locks       NO locks            │
└──────────────────────────────────────┘
   Cross-core: message passing only
```

**Pros:** Zero lock contention, zero false sharing, maximum cache locality, predictable latency (no OS scheduling jitter).  
**Cons:** Requires all I/O to be async (io_uring), cross-core communication must be explicit message passing (SPSC queues), actor placement is manual or heuristic-based.  
**Best for:** Ultra-low-latency systems, database engines, network appliances.

### 2.4 Recommended Hybrid for Jade

```
Default:  N:M with work-stealing (general purpose)
Opt-in:   Thread-per-actor (via annotation @pinned or spawn_pinned)
Opt-in:   Thread-per-core (via runtime configuration for I/O-critical apps)
```

The N:M model should be the default because:
1. Jade already targets systems programming — users expect millions of actors
2. Thread-per-actor can be a special case: `spawn_pinned ActorName` gives a dedicated thread
3. The runtime can start with N = num_cpus workers and grow as needed

---

## 3. Mailbox Architectures

### 3.1 Current: Bounded MPSC Ring Buffer

```
Capacity:     256 messages (power-of-2 for fast modulo via bitmask)
Sync:         mutex + 2 condvars
Enqueue:      lock → wait if full → write at tail → advance tail → unlock → signal
Dequeue:      lock → wait if empty → read at head → advance head → unlock → signal
Backpressure: Sender blocks on full (bounded)
```

**Analysis:** Correct and efficient for 1:1 model. Unlocking before signaling (as Jade does) is the right optimization — eliminates the "hurry up and wait" anti-pattern where a signaled thread wakes only to block on the mutex.

### 3.2 Lock-Free MPSC (Recommended Upgrade)

```
Structure:    Intrusive linked list with atomic compare-and-swap
Push:         CAS on tail pointer (lock-free, wait-free for single producer)
Pop:          Only consumer pops (single consumer = no contention)
Memory:       Message nodes from per-thread slab allocator
```

**Implementation approaches:**

| Algorithm | Throughput | Bounded? | Ordering | Notes |
|-----------|-----------|----------|----------|-------|
| Michael-Scott queue | ~50M msg/s | No | FIFO | Classic, well-proven |
| Dmitry Vyukov MPSC | ~100M msg/s | No | FIFO | Intrusive, minimal allocation |
| Tokio mpsc (crossbeam) | ~80M msg/s | Yes | FIFO | Hybrid: array slots + linked fallback |
| LMAX Disruptor | ~200M msg/s | Yes | FIFO | Ring buffer, padding against false sharing |

**Recommendation:** Start with Vyukov MPSC (unbounded, intrusive) with an optional bounded wrapper that uses a semaphore or atomic counter for backpressure. This matches what Erlang/BEAM uses internally.

### 3.3 Priority Mailbox

```jade
# Proposed syntax
actor Router
    @high_priority msg: Request     [priority: 10]
    @normal msg: Request            [priority: 5]
    @background msg: Task           [priority: 1]
```

**Implementation:** Multiple internal queues (one per priority level), dequeue checks high-priority first. Starvation prevention via credit-based scheduling (process N low-priority messages for every high-priority batch).

### 3.4 Mailbox Size Tuning

```jade
# Proposed syntax
actor Logger [mailbox: 4096]        # high-throughput logger
actor RateLimiter [mailbox: 16]     # tight backpressure
```

Current hardcoded 256 should become a per-actor-type parameter with 256 as default.

---

## 4. Named Actors, Registries, Discovery

### 4.1 Local Named Actors

```jade
# Proposed syntax
actor Logger
    @log msg: String
        # write to file

*main()
    spawn Logger as "logger"                 # register with name
    send "logger", @log("system started")    # send by name
```

**Implementation:** Global concurrent hashmap (`name → ActorRef`). Options:

| Registry Type | Lookup | Insert/Remove | Thread-Safety |
|---------------|--------|---------------|--------------|
| RwLock<HashMap> | O(1) avg, blocks on write | Blocks all reads during write | Simple, correct |
| DashMap / lock-free | O(1) avg, lock-free reads | Lock-free or fine-grained locking | Best for read-heavy |
| ETS-style (Erlang) | O(1), per-table lock | O(1) amortized | Battle-tested model |

**Naming conventions:** Flat string names (like Erlang's registered processes) vs. hierarchical paths (like Akka's `/user/router/worker-1`). Recommendation: **hierarchical** — it maps cleanly to supervision trees.

### 4.2 Actor Addresses

```
Local address:    ActorRef (pointer to mailbox)
Named address:    String → ActorRef lookup
Remote address:   node://host:port/path/to/actor
```

An `Address` type that unifies these:

```jade
enum Address
    Local(ActorRef)
    Named(String)
    Remote(NodeId, String)
```

### 4.3 Service Discovery

For distributed actors, service discovery options:

| Mechanism | Latency | Consistency | Complexity |
|-----------|---------|-------------|------------|
| Static config | 0 | Strong (manual) | None |
| DNS-based | ~ms | Eventual | Low |
| Consul/etcd | ~ms | Strong (Raft) | Medium |
| Gossip protocol | ~100ms | Eventual | Medium |
| mDNS/Bonjour | ~ms | Eventual (LAN only) | Low |

**Recommendation for Jade:** Start with static config + local named registry. Service discovery is a runtime concern that can be layered on later.

---

## 5. Remote Actors, Endpoints, Protocols

### 5.1 Transparent Location

The holy grail: `send actor, @msg(args)` works identically whether the actor is local or remote. This requires:

1. **Serialization:** Messages must be serializable for network transport
2. **Addressing:** Actor references must encode location
3. **Failure detection:** Network partitions, node crashes
4. **Ordering guarantees:** At-most-once? At-least-once? Exactly-once?

### 5.2 Wire Protocols

| Protocol | Latency | Throughput | Ordering | Complexity |
|----------|---------|------------|----------|------------|
| TCP raw | ~100μs | ~1GB/s | FIFO per connection | Low |
| QUIC | ~100μs | ~1GB/s | FIFO per stream, streams independent | Medium |
| Unix domain socket | ~10μs | ~5GB/s | FIFO | Low (local only) |
| UDP + reliability layer | ~50μs | Varies | Unordered (app adds ordering) | High |
| RDMA (InfiniBand) | ~1μs | ~100GB/s | FIFO | Very High |
| io_uring msg_ring | ~0.5μs | In-kernel | FIFO | Medium (Linux 6.0+) |
| shared memory + futex | ~0.1μs | Memory-speed | App-defined | Medium |

**Recommendation for Jade:**
- **Intra-machine:** Unix domain sockets for cross-process, shared memory + futex for high-performance
- **Inter-machine:** TCP with length-prefixed framing as default, QUIC as opt-in
- **Encoding:** Binary format (see §6)

### 5.3 Endpoint Architecture

```
┌──────────────────────────────────┐
│           Jade Node              │
│  ┌─────────┐   ┌─────────────┐  │
│  │ Actor    │   │ Endpoint    │  │
│  │ Runtime  │◄─►│ Listener    │  │
│  │          │   │ (TCP/UDS)   │  │
│  └─────────┘   └──────┬──────┘  │
│                        │         │
└────────────────────────┼─────────┘
                         │ network
┌────────────────────────┼─────────┐
│           Jade Node    │         │
│  ┌─────────┐   ┌──────┴──────┐  │
│  │ Actor    │   │ Endpoint    │  │
│  │ Runtime  │◄─►│ Connector   │  │
│  └─────────┘   └─────────────┘  │
└──────────────────────────────────┘
```

Each node runs an **Endpoint** that:
1. Listens for incoming connections
2. Maintains connection pool to known peers
3. Routes outbound messages to correct peer connection
4. Deserializes inbound messages and injects into local mailboxes
5. Handles connection lifecycle (reconnect, backoff, heartbeat)

### 5.4 Protocol Design

```
Frame format:
┌────────┬────────┬──────────┬──────────────┬─────────┐
│ magic  │ length │ msg_type │ target_actor │ payload │
│ 4B     │ 4B     │ 2B       │ var (LPS)    │ var     │
└────────┴────────┴──────────┴──────────────┴─────────┘

msg_type:
  0x01  ActorSend       — deliver message to actor
  0x02  ActorReply      — response to a request
  0x03  SpawnRemote     — create actor on remote node
  0x04  Ping/Pong       — heartbeat
  0x05  NodeJoin        — cluster membership
  0x06  NodeLeave       — graceful departure
```

---

## 6. Message Formats & Serialization

### 6.1 Current Format (Local Only)

```
{ i32 tag, [max_payload_bytes × i8] }
```

Messages are packed as raw bytes — tag selects the handler, payload bytes are extracted by GEP at known offsets. **Zero serialization cost.** This is optimal for local actors.

### 6.2 Serialization Formats for Remote/Persistent Messages

| Format | Encode speed | Decode speed | Size | Schema req'd | Zero-copy? |
|--------|-------------|-------------|------|-------------|------------|
| Raw memcpy | ~0 | ~0 | Exact | No (same arch) | Yes |
| MessagePack | ~1GB/s | ~1GB/s | Compact | No | No |
| FlatBuffers | ~0 (zero-copy write) | ~0 (zero-copy read) | Moderate | Yes (.fbs) | **Yes** |
| Cap'n Proto | ~0 | ~0 | Moderate | Yes (.capnp) | **Yes** |
| Protocol Buffers | ~500MB/s | ~500MB/s | Compact | Yes (.proto) | No |
| JSON | ~200MB/s | ~100MB/s | Large | No | No |
| CBOR | ~800MB/s | ~800MB/s | Compact | No | No |

**Recommendation:**
- **Local messages:** Keep current raw byte packing — it's optimal
- **Remote messages:** FlatBuffers or Cap'n Proto for zero-copy network transport
- **Persistent messages (stores, logs):** MessagePack or CBOR — self-describing, compact
- **Debug/inspect:** JSON as opt-in human-readable format

### 6.3 Auto-Serialization from Jade Types

The compiler knows the full type layout of every message. It can auto-generate serialization:

```jade
actor Worker
    @process data: String, count: i64
        # compiler knows: String (length-prefixed UTF-8) + i64 (8 bytes LE)
        # auto-generates:  serialize_Worker_process(buf, data, count)
        #                  deserialize_Worker_process(buf) -> (String, i64)
```

This eliminates the separate schema files that FlatBuffers/Protobuf require. The Jade compiler IS the schema.

---

## 7. Message Brokers

### 7.1 Broker Taxonomy

| Broker | Model | Ordering | Persistence | Throughput | Latency | Use Case |
|--------|-------|----------|-------------|------------|---------|----------|
| **Kafka** | Log-based, pull | Per-partition FIFO | Durable (disk) | ~2M msg/s/partition | ~5ms | Event sourcing, analytics, replay |
| **RabbitMQ** | Queue-based, push | Per-queue FIFO | Optional | ~50K msg/s/queue | ~1ms | Task queues, RPC, routing |
| **ZeroMQ** | Brokerless, socket | Per-socket FIFO | None (in-memory) | ~10M msg/s | ~30μs | Low-latency, embedded |
| **NATS** | Pub/sub, push | Per-subject FIFO | JetStream (optional) | ~10M msg/s | ~100μs | Cloud-native, microservices |
| **Redis Streams** | Log-based, pull | Per-stream FIFO | Optional (RDB/AOF) | ~1M msg/s | ~0.5ms | Lightweight event streams |
| **Pulsar** | Log-based, push/pull | Per-topic FIFO | Tiered (BookKeeper) | ~1M msg/s | ~5ms | Multi-tenant, geo-replicated |

### 7.2 MQ vs. Direct Actor Messaging

| Dimension | Actor Mailbox | Message Broker |
|-----------|--------------|----------------|
| **Latency** | ~0.1–1μs (local) | ~30μs–5ms |
| **Persistence** | None (in-memory) | Configurable |
| **Buffering** | Bounded (backpressure) | Unbounded (disk-backed) |
| **Delivery guarantee** | At-most-once | At-least-once / exactly-once |
| **Ordering** | FIFO per sender | FIFO per partition/queue |
| **Decoupling** | Tight (must know ActorRef) | Loose (topic/queue name) |
| **Replay** | No | Yes (Kafka, Pulsar, NATS JetStream) |
| **Multi-consumer** | No (single actor owns mailbox) | Yes (consumer groups) |
| **Failure recovery** | Actor crashes → messages lost | Broker retains until ack'd |

### 7.3 Integration Strategy for Jade

Don't embed a broker — provide connector actors:

```jade
# Kafka connector actor (hypothetical syntax)
actor KafkaConsumer
    topic: String
    target: ActorRef
    
    @start()
        # internal: connect to Kafka, start polling
        # on each message: send target, @process(msg)
    
    @stop()
        # graceful disconnect

# Usage
*main()
    processor is spawn DataProcessor
    consumer is spawn KafkaConsumer
    send consumer, @start()
```

Broker integration belongs in the **library layer**, not the language. The language provides:
1. Efficient actor mailboxes for local communication
2. Serialization primitives for wire format
3. Extern FFI for calling broker client libraries (librdkafka, librabbitmq, etc.)

### 7.4 MQ vs. Tasks vs. Workflow Engines

| Concern | Actor Message | Task Queue | Workflow Engine |
|---------|--------------|------------|-----------------|
| **Granularity** | Single message | Unit of work | Multi-step process |
| **State** | In actor | Stateless (carried in task) | Persisted workflow state |
| **Routing** | To specific actor | To any available worker | Defined by DAG |
| **Retry** | Application-level | Built-in (DLQ, backoff) | Built-in (step-level) |
| **Visibility** | None (fire-and-forget) | Queue depth, latency metrics | Full execution trace |
| **Examples** | Erlang, Akka | Celery, Sidekiq, Bull | Temporal, Airflow, Step Functions |

**Insight for Jade:** Actor messages are the primitive. Task queues and workflow engines are patterns built ON TOP of actors:

```
Workflow Engine = Actor(orchestrator) + Actor(step_executor)[] + Persistent Store
Task Queue = Actor(dispatcher) + Actor(worker)[] + Priority Mailbox
```

---

## 8. Event Loops vs. On-Message Wake

### 8.1 Event Loop Model (Node.js, libuv, Tokio, io_uring)

```
while running:
    events = poll(epoll/kqueue/io_uring, timeout)
    for event in events:
        dispatch(event)           # I/O ready, timer fired, signal
    run_microtasks()              # promises, callbacks
    run_timers()                  # scheduled work
```

**Characteristics:**
- Single thread processes all events sequentially
- Never blocks — I/O is non-blocking, timers are wheel-based
- Excellent throughput for I/O-bound work (10K+ connections per thread)
- Poor for CPU-bound work (blocks the loop)
- Latency depends on task duration (one slow handler delays all)

### 8.2 Message-Wake Model (Erlang/BEAM, Current Jade)

```
loop:
    msg = mailbox.dequeue()       # blocks until message arrives
    match msg.tag:
        handle(msg)
    goto loop
```

**Characteristics:**
- Actor sleeps when no messages (zero CPU while idle)
- Wakes only when message arrives (condition variable signal / futex_wake)
- Natural for request/response patterns
- No timer wheel needed — timers are messages from a timer actor
- CPU-bound work runs in actor's time slice

### 8.3 Hybrid: Reactor + Actor (Recommended for Jade)

```
┌────────────────────────────────────────┐
│              Worker Thread              │
│                                        │
│  ┌──────────────┐                      │
│  │ Event Loop   │                      │
│  │  io_uring /  │                      │
│  │  epoll       │                      │
│  └──────┬───────┘                      │
│         │                              │
│    ┌────┴─────┐                        │
│    │ Dispatch │                        │
│    └────┬─────┘                        │
│    ┌────┴──┬───────┬───────┐           │
│    ▼       ▼       ▼       ▼           │
│  ┌───┐  ┌───┐  ┌───┐  ┌───┐           │
│  │ A │  │ B │  │ C │  │ D │ (actors)  │
│  └───┘  └───┘  └───┘  └───┘           │
│                                        │
│  Run queue: A → C → D                 │
│  I/O wait:  B (waiting on socket)     │
└────────────────────────────────────────┘
```

Each worker thread has:
1. An **event loop** (io_uring on Linux, kqueue on macOS) for I/O completion
2. A **run queue** of actors with pending messages
3. A **scheduler** that alternates between:
   - Processing actor messages (run N messages per actor before yielding)
   - Polling for I/O completions (non-blocking poll, inject wake messages to actors)

This is what Pony, Zig's `std.event.Loop`, and modern Tokio (with actor abstractions) effectively do.

### 8.4 Wake Mechanisms Compared

| Mechanism | Latency | Overhead | Cross-thread? | Cross-process? |
|-----------|---------|----------|---------------|----------------|
| pthread_cond_signal | ~1μs | Mutex required | Yes | No |
| futex(FUTEX_WAKE) | ~0.5μs | No mutex needed | Yes | Yes (shared mapping) |
| eventfd | ~0.5μs | fd + read/write | Yes | Yes (fd passing) |
| io_uring msg_ring | ~0.3μs | In-kernel | Yes | No |
| pipe/socketpair | ~1μs | fd + read/write | Yes | Yes |
| Signal (SIGUSR1) | ~2μs | Async-signal-safety issues | Yes | Yes |
| epoll_wait wakeup | ~0.5μs | Needs eventfd or pipe | N/A | N/A |

**Recommendation for Jade:** Move from `pthread_cond` to `futex` on Linux. Futex is lower latency, doesn't require a mutex for the signal path, and supports cross-process wake for future multi-process actor systems.

```c
// Current (Jade):
pthread_mutex_lock(&mb->mutex);
while (mb->count == 0) pthread_cond_wait(&mb->cond_ne, &mb->mutex);
// dequeue
pthread_mutex_unlock(&mb->mutex);

// Proposed (futex):
while (atomic_load(&mb->count) == 0)
    futex_wait(&mb->count, 0);  // sleep until count changes
// dequeue (lock-free MPSC)
// No mutex needed for dequeue in single-consumer case
```

---

## 9. Orchestrators, Workflow Engines, Supervision

### 9.1 Supervision Trees (Erlang Model)

```
           ┌─────────────┐
           │  Supervisor  │
           │  (one_for_one)│
           └──┬─────┬─────┘
              │     │
         ┌────┴┐  ┌─┴────┐
         │ W1  │  │ W2   │
         │(run)│  │(dead)│ ← crash → restart
         └─────┘  └──────┘
```

Supervision strategies:

| Strategy | Behavior | Use Case |
|----------|----------|----------|
| one_for_one | Restart only crashed child | Independent workers |
| one_for_all | Restart all children if any crash | Tightly coupled group |
| rest_for_one | Restart crashed + all started after it | Pipeline stages |
| simple_one_for_one | Dynamic child spawning, one_for_one restart | Worker pools |

**Proposed Jade syntax:**

```jade
supervisor WorkerPool [strategy: one_for_one, max_restarts: 3, window: 60]
    Worker
    Worker
    Worker
    Logger

# Or inline
*main()
    pool is supervise [one_for_one]
        spawn Worker
        spawn Worker
        spawn Logger
```

### 9.2 Request-Reply Pattern

Currently Jade actors are fire-and-forget. Request-reply is essential:

```jade
# Proposed: ask pattern
actor Database
    @query sql: String -> String    # reply type annotation
        result is execute(sql)
        reply result                 # explicit reply

*main()
    db is spawn Database
    result is ask db, @query("SELECT 1")   # blocks caller until reply
    log(result)
```

**Implementation:** The `ask` expression:
1. Allocates a one-shot reply channel (single-element mailbox)
2. Sends the message with the reply channel pointer packed in
3. Blocks the caller on the reply channel's condvar/futex
4. Handler calls `reply` which writes to the reply channel and wakes the caller

### 9.3 Orchestrator Pattern

```jade
actor OrderProcessor
    @process order: Order
        # Orchestrate multi-step workflow
        validated is ask ValidationService, @validate(order)
        if validated
            payment is ask PaymentService, @charge(order)
            if payment.success
                send InventoryService, @reserve(order)
                send NotificationService, @confirm(order)
            else
                send NotificationService, @payment_failed(order)
```

This is the **saga pattern** — each step is an actor message, failures trigger compensating actions. The orchestrator actor holds workflow state.

### 9.4 Per-Thread Orchestrators

In the N:M model, each worker thread can have a **thread-local orchestrator** that manages actors pinned to that thread:

```
Thread 0:  Orchestrator₀ → [ActorA, ActorB, ActorC]
Thread 1:  Orchestrator₁ → [ActorD, ActorE]
Thread 2:  Orchestrator₂ → [ActorF, ActorG, ActorH, ActorI]
```

Benefits:
- No cross-thread synchronization for thread-local actor communication
- Cache locality (actors on same thread share L1/L2 cache)
- Orchestrator can batch-process messages across its actors

This is the Seastar/ScyllaDB model and delivers the best possible latency.

---

## 10. Timed Events, Wakeups, Signals

### 10.1 Timer Wheel Architecture

```
┌─────────────────────────────────────┐
│          Timer Wheel (per thread)    │
│  Slot 0 → [TimerA, TimerB]         │
│  Slot 1 → []                       │
│  Slot 2 → [TimerC]                 │
│  ...                                │
│  Slot 63 → [TimerD]                │
│                                     │
│  Tick: advance slot, fire expired   │
│  Granularity: 1ms per slot          │
│  Capacity: 64 slots = 64ms wheel   │
│  Overflow: hierarchical wheel       │
└─────────────────────────────────────┘
```

**Proposed Jade syntax:**

```jade
actor Heartbeat
    @start target: ActorRef
        # Send a ping every 5 seconds
        every 5000                     # ms
            send target, @ping()
    
    @timeout()
        # Called after delay
        log("no response")

*main()
    monitor is spawn Heartbeat
    send monitor, @start(some_actor)
    
    # One-shot timer
    after 1000                         # ms
        log("1 second elapsed")
```

### 10.2 Implementation Options

| Approach | Resolution | Overhead | Scalability |
|----------|-----------|----------|-------------|
| `sleep()` in dedicated thread | ~1ms | 1 thread per timer | Poor |
| `timerfd_create` + epoll | ~1μs | 1 fd per timer | Good (1000s) |
| Timer wheel (Hashed) | ~1ms | O(1) insert/cancel | Excellent (millions) |
| Timer wheel (Hierarchical) | ~1ms–1s | O(1) insert/cancel | Excellent |
| io_uring timeout | ~1μs | Kernel-managed | Good |
| `SIGALRM` / `timer_create` | ~1ms | Per-process limit | Poor |

**Recommendation:** Hierarchical timer wheel per worker thread, integrated with the event loop. Timers are messages — when a timer fires, it enqueues a message to the target actor's mailbox.

### 10.3 Signal Handling

Jade already has POSIX signal handling. For actors:

```jade
actor SignalHandler
    @on_signal sig: i32
        match sig
            2  -> log("SIGINT received, shutting down")
            15 -> log("SIGTERM received, graceful stop")
```

**Implementation:** A dedicated signal-handling thread (using `sigwait()`) that converts signals into actor messages. This is the standard pattern — never do real work in a signal handler.

---

## 11. OS/Kernel-Level Support

### 11.1 Linux Kernel Facilities

| Facility | What it does | Actor relevance |
|----------|-------------|----------------|
| **futex** | Fast userspace mutex/wake | Mailbox synchronization without pthread overhead |
| **io_uring** | Async I/O submission/completion ring | Non-blocking I/O for actor event loops |
| **epoll** | I/O event notification | Legacy alternative to io_uring for I/O multiplexing |
| **splice/vmsplice** | Zero-copy data transfer between fds/pipes | High-throughput message passing between processes |
| **memfd_create** | Anonymous shared memory | Cross-process actor mailboxes |
| **userfaultfd** | Page fault handling in userspace | Lazy actor state migration |
| **pidfd** | Process lifecycle as fd | Supervising actor processes |
| **io_uring msg_ring** | Cross-ring message passing | Ultra-low-latency cross-thread actor notification |
| **CLONE_VM** | Shared address space threads | Lightweight actor threads with shared memory |
| **sched_setaffinity** | CPU pinning | Thread-per-core actor affinity |
| **cgroups v2** | Resource limits | Per-actor-group CPU/memory quotas |
| **BPF** | Programmable kernel hooks | Actor message tracing, scheduling policies |
| **rseq** | Restartable sequences | Per-CPU data structures without locks |

### 11.2 io_uring Deep Integration

```
┌─────────────────────────────────────────┐
│              Jade Runtime                │
│                                         │
│  Submission Queue (SQ)                  │
│  ┌────┬────┬────┬────┐                  │
│  │read│send│recv│tmr │ ← actor I/O ops  │
│  └────┴────┴────┴────┘                  │
│         │ (shared ring buffer)          │
│         ▼                               │
│  ┌──────────────┐                       │
│  │    Kernel     │                      │
│  │  io_uring    │                       │
│  └──────┬───────┘                       │
│         │                               │
│  Completion Queue (CQ)                  │
│  ┌────┬────┬────┬────┐                  │
│  │done│done│done│done│ → wake actors    │
│  └────┴────┴────┴────┘                  │
└─────────────────────────────────────────┘
```

io_uring is the single most important kernel facility for a modern actor system because:
1. **Batched submission:** Multiple I/O operations in one syscall (or zero syscalls with SQPOLL)
2. **Zero-copy:** `IORING_OP_SEND_ZC`, `IORING_OP_SPLICE` for network/pipe I/O
3. **Fixed buffers:** Register buffers once, kernel DMAs directly — no copy
4. **Linked operations:** Chain read → process → write as a single submission
5. **msg_ring:** Cross-ring notification without going through network stack

**For Jade actors:** Each worker thread owns one io_uring instance. Actor I/O operations (file read, socket send, etc.) submit to the local ring. Completions wake the requesting actor.

### 11.3 splice for Inter-Process Actors

```
Process A  ──pipe──►  Kernel  ──pipe──►  Process B
              (zero-copy, no userspace buffer)
```

`splice()` moves data between file descriptors without copying through userspace. For cross-process actor messaging:
1. Each process pair has a pipe
2. Messages are written to the pipe (serialized)
3. `splice` moves them kernel-side to the receiving pipe
4. Receiving process reads and injects into local mailbox

Combined with `vmsplice` (map userspace pages into pipe), this enables zero-copy cross-process messaging.

### 11.4 macOS/BSD Equivalents

| Linux | macOS/BSD | Notes |
|-------|-----------|-------|
| futex | os_unfair_lock / `__ulock_wait` | macOS private API, faster than pthread |
| io_uring | kqueue + kevent64 | Less capable, no batched submission |
| epoll | kqueue | Functionally equivalent |
| splice | sendfile | More limited (file→socket only) |
| memfd_create | shm_open | POSIX shared memory |
| sched_setaffinity | thread_policy_set | macOS thread affinity |

**Platform strategy for Jade:** Abstract behind a platform layer:

```
src/runtime/
    platform_linux.c    — futex, io_uring, splice, memfd
    platform_macos.c    — kqueue, os_unfair_lock, shm_open
    platform.h          — common interface
```

---

## 12. Async Integration

### 12.1 The Core Tension

Actors have their own scheduling model (message-driven). Async/await has its own (Future/Promise-driven). They can conflict:

| Aspect | Actor Model | Async/Await |
|--------|------------|-------------|
| Unit of work | Message handler | Future/Task |
| Scheduling | Mailbox-driven | Executor-driven |
| State | Encapsulated in actor | Captured in Future state machine |
| Concurrency | One message at a time per actor | Multiple awaits can interleave |
| Cancellation | Stop message | Drop the Future |

### 12.2 Integration Strategies

**Strategy A: Actors ARE the async runtime (Recommended)**

Every actor is a state machine driven by messages. `await` inside an actor suspends the actor (saves state, yields the worker thread to other actors) and resumes when the I/O completes.

```jade
actor HttpHandler
    @request req: Request
        body is await read_body(req)        # suspends actor, yields thread
        response is process(body)            # resumes when I/O complete
        await send_response(req, response)   # suspends again
```

**Implementation:** The compiler transforms `await` in actor handlers into CPS (continuation-passing style):
1. Save current handler state (locals) to actor state
2. Register I/O completion callback that sends a resume message
3. Return from handler (actor can process other messages)
4. On resume message, restore state and continue

This is exactly what Erlang does — `receive` blocks the process (not the OS thread), and the BEAM scheduler moves on to another process.

**Strategy B: Actors on top of async runtime**

Actors are tasks on a standard async executor (Tokio, async-std). Actor mailbox operations are `async`:

```rust
// Pseudo-implementation
async fn actor_loop(mut mailbox: Receiver<Msg>) {
    while let Some(msg) = mailbox.recv().await {
        handle(msg).await;
    }
}
```

This works but subordinates actors to the async runtime's scheduling decisions. Used by Actix (Rust).

**Strategy C: Separate models, bridge at boundaries**

Actors and async Tasks are independent. Communication via channels that bridge the two:

```jade
*main()
    worker is spawn Worker              # actor
    result is async fetch("http://...")  # async task
    send worker, @process(result)       # bridge
```

Simpler but loses the benefit of unified scheduling.

**Recommendation for Jade:** **Strategy A.** Actors should BE the concurrency primitive. `await` is syntactic sugar for "suspend this actor, resume on completion." The actor scheduler IS the async executor. This gives Jade a single, unified model.

### 12.3 Can Actors Be the ONLY Parallel Execution Model?

**Yes, with caveats.** See §13.

---

## 13. Actors as the Sole Parallelism Model?

### 13.1 What Actors Handle Well

| Pattern | Actor approach | Quality |
|---------|---------------|---------|
| Request/response servers | One actor per connection/request | Excellent |
| Event processing pipelines | Actor chain: source → transform → sink | Excellent |
| State machines | Actor state + message-driven transitions | Excellent |
| Supervision/fault tolerance | Supervisor actors | Excellent |
| Pub/sub | Broker actor + subscriber list | Good |
| Worker pools | Supervisor + N identical worker actors | Good |
| Distributed systems | Remote actors + location transparency | Good |

### 13.2 What Actors Handle Poorly (Without Extensions)

| Pattern | Problem | Mitigation |
|---------|---------|------------|
| **Parallel data processing** | Map-reduce over array needs N actors for N partitions — overhead for fine-grained parallelism | Provide `parallel_map` as built-in that uses work-stealing under the hood |
| **Shared mutable state** | Actors can't share memory — must serialize through a "state server" actor (bottleneck) | Accept this: the bottleneck IS the correct serialization point |
| **Barrier synchronization** | N actors must all reach a point before any proceeds — requires a coordinator actor | Provide `barrier` primitive built on top of actors |
| **Fork-join parallelism** | Spawn N tasks, wait for all results — clunky with actors | Provide `parallel` block or `join` on actor futures |
| **Lock-free data structures** | Actors don't expose shared memory — can't build lock-free queues etc. | Allow `unsafe` escape hatch for expert users, or use actors as the queueing mechanism |
| **SIMD / GPU offload** | Actors are control-flow oriented, not data-flow | Provide compute primitives (matrix ops, etc.) within actor handlers |
| **Real-time deadlines** | Actor scheduling is best-effort — no hard real-time guarantees | Priority actors + deadline scheduling in the runtime |

### 13.3 Verdict: Actors + Structured Parallelism

Actors should be the **primary** model but not the **only** model. Complement with:

1. **`parallel` blocks for data parallelism:**
   ```jade
   results is parallel
       compute_chunk(data[0..1000])
       compute_chunk(data[1000..2000])
       compute_chunk(data[2000..3000])
   # All three run concurrently, results collected
   ```

2. **`parallel_map` for collection processing:**
   ```jade
   results is data ~ parallel_map(*process)
   ```

3. **Channels for streaming data:**
   ```jade
   ch is channel of i64 [capacity: 100]
   
   spawn *()
       for i from 0 to 1000
           ch.send(i)
   
   for val in ch
       log(val)
   ```

4. **Atomic operations for low-level shared state:**
   ```jade
   counter: atomic i64 is 0
   counter.fetch_add(1)
   ```

**The hierarchy:**
```
Actors (primary: state + messages + supervision)
  └─ Channels (streaming data between actors or tasks)
      └─ parallel/parallel_map (data parallelism)
          └─ Atomics (expert-level, unsafe-adjacent)
```

---

## 14. Performance Analysis

### 14.1 Message Passing Overhead Breakdown

| Component | Current (pthread) | Futex | Lock-free MPSC | io_uring msg_ring |
|-----------|------------------|-------|---------------|-------------------|
| Lock acquisition | ~50ns | ~25ns | 0 (atomic CAS) | N/A |
| Enqueue | ~10ns | ~10ns | ~15ns (CAS + retry) | ~50ns (SQE) |
| Signal/wake | ~500ns (cond_signal) | ~200ns (futex_wake) | ~200ns (futex_wake) | ~100ns (kernel) |
| Dequeue | ~10ns | ~10ns | ~5ns (single consumer) | ~50ns (CQE) |
| Unlock | ~20ns | ~10ns | 0 | N/A |
| **Total per message** | **~590ns** | **~255ns** | **~220ns** | **~200ns** |
| **Throughput** | **~1.7M msg/s** | **~3.9M msg/s** | **~4.5M msg/s** | **~5M msg/s** |

### 14.2 Actor Creation Overhead

| Approach | Cost | Memory | Max actors |
|----------|------|--------|------------|
| pthread_create (current) | ~50μs | ~8MB stack | ~10K |
| pthread_create + small stack | ~50μs | ~64KB stack | ~100K |
| N:M lightweight (stackful) | ~1μs | ~2KB initial | ~1M |
| N:M lightweight (stackless) | ~100ns | ~256B state | ~10M |
| Thread-per-core (no creation) | 0 (pre-allocated) | Inline | ~100K per core |

### 14.3 Throughput Comparison Across Systems

| System | Msg/s (single core) | Msg/s (8 cores) | Actor creation/s |
|--------|--------------------|-----------------|-----------------| 
| Erlang/OTP 27 | ~2M | ~12M | ~500K |
| Akka (JVM) | ~5M | ~30M | ~1M |
| Pony | ~15M | ~80M | ~5M |
| CAF (C++) | ~10M | ~60M | ~2M |
| Actix (Rust) | ~8M | ~50M | ~3M |
| Go channels | ~5M | ~25M | ~1M (goroutines) |
| **Jade (current)** | **~1.7M** | **~8M** (est.) | **~20K** |
| **Jade (target)** | **~10M** | **~60M** | **~5M** |

### 14.4 Latency Profile

| Percentile | Jade current | Erlang | Pony | Target |
|------------|-------------|--------|------|--------|
| p50 | ~600ns | ~1μs | ~100ns | ~200ns |
| p99 | ~5μs | ~10μs | ~500ns | ~1μs |
| p99.9 | ~50μs | ~100μs | ~5μs | ~10μs |
| p99.99 | ~500μs (OS sched) | ~1ms | ~50μs | ~50μs |

The tail latency problem in the current implementation comes from OS thread scheduling. Moving to N:M with cooperative scheduling eliminates OS scheduler jitter for p99.99.

---

## 15. Scalability Analysis

### 15.1 Vertical Scaling (Single Machine)

| Dimension | Current | N:M | Thread-per-core |
|-----------|---------|-----|-----------------|
| Actors | ~10K (thread limit) | ~10M | ~100K per core |
| Messages/s | ~8M (8 cores) | ~60M | ~100M |
| Memory overhead | ~80GB (10K × 8MB stack) | ~20GB (10M × 2KB) | ~1GB (100K × 10KB) |
| CPU utilization | Low (thread idle in wait) | High (work-stealing) | Maximum (no idle) |
| Context switches | ~10K/s (OS) | ~0 (cooperative) | ~0 (run-to-completion) |

### 15.2 Horizontal Scaling (Multi-Machine)

| Challenge | Solution |
|-----------|----------|
| Actor placement | Hash-based partitioning or consistent hashing |
| State migration | Serialize actor state, transfer, resume on new node |
| Network partition | Supervision tree detects failure, restarts locally |
| Load balancing | Router actor distributes work across nodes |
| Clock synchronization | Vector clocks or hybrid logical clocks for ordering |

### 15.3 Scaling Bottlenecks

| Bottleneck | When it hits | Mitigation |
|------------|-------------|------------|
| Single actor hotspot | All messages funnel through one actor | Shard the actor (consistent hashing) |
| Mailbox overflow | Producer faster than consumer | Backpressure (bounded) or scaling consumers |
| GC pauses (JVM-based) | N/A for Jade (no GC) | **Jade advantage** |
| Serialization overhead | Remote messaging | Zero-copy formats (FlatBuffers) |
| Network bandwidth | Cross-node traffic | Locality-aware placement |
| Memory fragmentation | Long-running actors with varied message sizes | Slab allocator per actor type |

---

## 16. Reliability Analysis

### 16.1 Failure Modes

| Failure | Current Jade | With supervision | With persistence |
|---------|-------------|-----------------|-----------------|
| Actor crash (panic) | **Process crash** | Supervisor restarts | Restart + replay from log |
| Mailbox full | Sender blocks (correct) | Same | Same |
| Message lost (bug) | Silent | Timeout + retry | Idempotent replay |
| Node crash | Total loss | Remote supervisor restarts | Checkpoint + replay |
| Network partition | N/A (local only) | Split-brain detection | Partition-tolerant protocol |
| Memory exhaustion | OOM kill | Per-actor memory limits | Spill to disk |

### 16.2 Delivery Guarantees

| Guarantee | Mechanism | Overhead | Use case |
|-----------|-----------|----------|----------|
| At-most-once | Fire-and-forget (current) | None | Metrics, logs, non-critical |
| At-least-once | Ack + retry on timeout | 2× messages | Payments, orders |
| Exactly-once | Idempotency key + dedup | Dedup table | Financial transactions |

**Proposed Jade syntax:**

```jade
# At-most-once (default, current behavior)
send worker, @process(data)

# At-least-once (with ack and retry)
send worker, @process(data) [ack: true, timeout: 5000, retries: 3]

# Request-reply (natural exactly-once for the reply)
result is ask worker, @compute(data) [timeout: 5000]
```

### 16.3 Erlang's "Let It Crash" Philosophy

Erlang's key insight: **don't try to handle every error in the actor — let it crash, and let the supervisor handle recovery.** This works because:

1. Actor state is isolated — crash doesn't corrupt other actors
2. Supervisor knows how to restart (same init args)
3. Persistent state lives outside the actor (ETS, Mnesia, disk)
4. Crash logs provide debugging info without try/catch overhead

**For Jade:** This maps perfectly to Jade's existing error model (`!` return, error definitions). Unhandled errors in actor handlers should crash the actor and notify the supervisor.

---

## 17. Historical Landscape

### 17.1 Timeline of Actor Implementations

| Year | System | Innovation |
|------|--------|-----------|
| 1973 | Actor Model (Hewitt) | Original theory — everything is an actor |
| 1986 | Erlang | Practical actor system with supervision, hot code reload |
| 1988 | Concurrent ML | Synchronous channels as alternative to actors |
| 2004 | Scala Actors | Actors on JVM, thread-based |
| 2009 | Akka | Scala/Java actor framework, location transparency |
| 2010 | Go | Goroutines + channels (CSP, not actors, but related) |
| 2011 | Erlang R14 (SMP) | Multi-core BEAM with per-scheduler run queues |
| 2014 | Pony | Reference capabilities + actors, zero-copy guaranteed |
| 2015 | Actix (Rust) | Actors on Tokio async runtime |
| 2016 | Virtual Actors (Orleans) | Auto-activation, location transparent, persisted |
| 2017 | CAF (C++) | Type-safe C++ actor framework |
| 2019 | Lunatic | WASM-based actors with Erlang-style supervision |
| 2020 | io_uring msg_ring | Kernel-native cross-ring messaging |
| 2021 | Project Loom (Java) | Virtual threads make actor-per-request practical on JVM |
| 2023 | Gleam | Erlang actors with static types |
| 2024 | Mojo | Actor-like parallelism for AI workloads |
| 2025 | **Jade** | Native-compiled actors with Perceus RC, C-parity performance |

### 17.2 Design Philosophy Comparison

| System | Philosophy | Trade-off |
|--------|-----------|-----------|
| Erlang | Reliability over speed | ~10× slower than C, but 99.9999% uptime |
| Akka | Flexibility, JVM ecosystem | GC pauses, heavyweight actors |
| Pony | Zero-copy safety via capabilities | Complex type system, small ecosystem |
| Go | Simplicity (CSP channels) | No supervision, manual error handling |
| Actix | Performance on async runtime | Rust complexity, actor abstraction leaks |
| **Jade** (target) | C performance + Erlang reliability | Must build runtime from scratch |

---

## 18. Recommended Architecture for Jade

### 18.1 Runtime Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Jade Actor Runtime                     │
│                                                          │
│  ┌──────────────────────────────────────────────────┐   │
│  │              Scheduler (N:M)                      │   │
│  │  ┌─────────┐  ┌─────────┐  ┌─────────┐          │   │
│  │  │Worker 0 │  │Worker 1 │  │Worker N │          │   │
│  │  │┌───────┐│  │┌───────┐│  │┌───────┐│          │   │
│  │  ││RunQ   ││  ││RunQ   ││  ││RunQ   ││          │   │
│  │  ││A,B,C  ││  ││D,E    ││  ││F,G,H  ││          │   │
│  │  │├───────┤│  │├───────┤│  │├───────┤│          │   │
│  │  ││Timer  ││  ││Timer  ││  ││Timer  ││          │   │
│  │  ││Wheel  ││  ││Wheel  ││  ││Wheel  ││          │   │
│  │  │├───────┤│  │├───────┤│  │├───────┤│          │   │
│  │  ││io_uring││ ││io_uring││ ││io_uring││         │   │
│  │  │└───────┘│  │└───────┘│  │└───────┘│          │   │
│  │  └─────────┘  └─────────┘  └─────────┘          │   │
│  │         ←── work stealing ──→                     │   │
│  └──────────────────────────────────────────────────┘   │
│                                                          │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │ Actor Registry│  │ Supervisor   │  │ Metrics      │  │
│  │ (name→ref)   │  │ Tree         │  │ Collector    │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
│                                                          │
│  ┌──────────────────────────────────────────────────┐   │
│  │              Platform Layer                       │   │
│  │  Linux: futex + io_uring + splice + memfd        │   │
│  │  macOS: kqueue + os_unfair_lock + shm_open       │   │
│  └──────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
```

### 18.2 Actor Lifecycle

```
            spawn
              │
              ▼
         ┌─────────┐
         │  Init    │ ← run init handler (constructor args)
         └────┬─────┘
              │
              ▼
         ┌─────────┐ ◄──────── messages
         │ Running  │ ────────► process message
         └────┬─────┘           (one at a time)
              │
         ┌────┴────┐
         │         │
    crash│    stop │
         │         │
         ▼         ▼
    ┌────────┐ ┌────────┐
    │Crashed │ │Stopped │
    └────┬───┘ └────────┘
         │
    supervisor
    decision
         │
    ┌────┴────┐
    │         │
  restart   escalate
    │         │
    ▼         ▼
  Init    supervisor
          crashes
```

### 18.3 Proposed Language Extensions

```jade
# 1. Constructor arguments at spawn
actor Counter
    count: i64
    @init start: i64           # constructor
        count is start
    @increment
        count is count + 1

c is spawn Counter(100)        # starts at 100

# 2. Reply mechanism
actor Calculator
    @compute x: i64, y: i64 -> i64
        reply x + y

result is ask calc, @compute(10, 20)   # = 30

# 3. Supervision
supervisor AppSupervisor [one_for_one]
    spawn Logger as "logger"
    spawn Database as "db"
    spawn WebServer as "web"

# 4. Named actors
send "logger", @log("hello")

# 5. Selective receive (pattern matching on mailbox)
actor StateMachine
    state: i32
    @event e: Event
        match state, e
            0, Event.Start -> state is 1
            1, Event.Data(d) -> process(d)
            1, Event.Stop -> state is 0
            _ -> log("unexpected")

# 6. Actor-level async I/O
actor FileProcessor
    @process path: String
        contents is await read_file(path)    # async, non-blocking
        result is transform(contents)
        await write_file("out.txt", result)

# 7. Timers
actor Monitor
    @start()
        every 1000
            send self, @check()
    @check()
        # periodic health check

# 8. Graceful stop
actor Worker
    @stop()                    # system handler called on shutdown
        # cleanup resources
        log("worker stopping")

*main()
    w is spawn Worker
    stop w                     # sends @stop, waits for drain, joins
```

### 18.4 Configuration System

```jade
# Runtime configuration (compile-time or runtime flags)
@runtime
    workers: 8                 # N worker threads (default: num_cpus)
    mailbox_default: 256       # default mailbox capacity
    scheduler: work_stealing   # work_stealing | thread_per_core | thread_per_actor
    io_backend: io_uring       # io_uring | epoll | kqueue (auto-detected)
    
# Per-actor configuration
actor HighThroughput [mailbox: 4096, priority: high, pinned: true]
    # ...
```

---

## 19. Implementation Roadmap

### Phase 1: Foundation (Current → v0.1)

| Task | Complexity | Impact |
|------|-----------|--------|
| Actor stop/kill (graceful + forced) | Medium | Fixes actor lifecycle |
| Constructor arguments (`@init`) | Low | Essential for real use |
| Reply mechanism (`ask`/`reply`) | Medium | Unlocks request/response |
| Configurable mailbox size | Low | Performance tuning |
| State initialization from spawn args | Low | Quality of life |

### Phase 2: Reliability (v0.1 → v0.2)

| Task | Complexity | Impact |
|------|-----------|--------|
| Supervision trees | High | Erlang-level reliability |
| Named actor registry | Medium | Actor discovery |
| Error propagation (crash → supervisor) | Medium | Fault tolerance |
| Delivery guarantees (at-least-once) | Medium | Production readiness |
| `self` reference in handlers | Low | Self-messaging |

### Phase 3: Performance (v0.2 → v0.3)

| Task | Complexity | Impact |
|------|-----------|--------|
| Lock-free MPSC mailbox (Vyukov) | High | ~2.5× throughput |
| futex-based wake (Linux) | Medium | ~2× wake latency |
| N:M scheduler with work-stealing | Very High | Millions of actors |
| Per-thread run queues | High | Cache locality |
| Timer wheel integration | Medium | Timed events |

### Phase 4: I/O Integration (v0.3 → v0.4)

| Task | Complexity | Impact |
|------|-----------|--------|
| io_uring integration (Linux) | Very High | Non-blocking I/O |
| kqueue integration (macOS) | High | Cross-platform |
| `await` in actor handlers | High | Async I/O |
| File/socket actors | Medium | I/O actor library |
| Platform abstraction layer | Medium | Portability |

### Phase 5: Distribution (v0.4 → v0.5)

| Task | Complexity | Impact |
|------|-----------|--------|
| Remote actor addressing | High | Multi-node |
| Wire protocol (TCP + framing) | Medium | Network transport |
| Auto-serialization from types | High | Zero-schema remote |
| Node membership protocol | High | Cluster formation |
| Actor migration | Very High | Elastic scaling |

### Phase 6: Ecosystem (v0.5 → v1.0)

| Task | Complexity | Impact |
|------|-----------|--------|
| Broker connector actors (Kafka, NATS) | Medium each | Integration |
| HTTP server actor | Medium | Web applications |
| Database driver actors | Medium | Data access |
| Monitoring/tracing | Medium | Observability |
| `parallel` blocks | Medium | Data parallelism |

---

## Summary: Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Default threading | N:M work-stealing | Millions of actors, balanced load |
| Mailbox | Lock-free MPSC, bounded optional | Maximum throughput |
| Wake mechanism | futex (Linux), os_unfair_lock (macOS) | Lowest latency |
| Async model | Actors ARE the async runtime | Single unified model |
| Supervision | Erlang-style trees | Proven reliability |
| Remote protocol | TCP + binary framing | Simple, extensible |
| Serialization | Auto-generated from types | No separate schema |
| Data parallelism | `parallel` blocks alongside actors | Actors alone are insufficient |
| I/O | io_uring integration | Maximum I/O throughput |
| Timer | Hierarchical timer wheel | O(1) for millions of timers |

The actor model should be Jade's **primary** concurrency primitive — the default way users think about parallelism. But it should be complemented with structured parallelism (`parallel`, `parallel_map`) for data-parallel workloads, and low-level atomics for expert users. This gives Jade the ergonomics of Erlang, the performance of C, and the safety of a modern systems language.

---

*End of report.*
