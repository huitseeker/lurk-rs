use std::time::Duration;

use criterion::{
    black_box, criterion_group, criterion_main, measurement, BenchmarkGroup, BenchmarkId,
    Criterion, SamplingMode,
};

use lurk::{
    eval::{
        empty_sym_env,
        lang::{Coproc, Lang},
        Evaluator,
    },
    field::LurkField,
    proof::nova::NovaProver,
    proof::Prover,
    ptr::Ptr,
    store::Store,
};

const DEFAULT_REDUCTION_COUNT: usize = 100;
fn fib<F: LurkField>(store: &mut Store<F>, a: u64) -> Ptr<F> {
    let program = format!(
        r#"
(let ((fib (lambda (target)
              (letrec ((next (lambda (a b target)
                               (if (= 0 target)
                                     a
                                     (next b
                                           (+ a b)
                                           (- target 1))))))
                (next 0 1 target)))))
  (fib {a}))
"#
    );

    store.read(&program).unwrap()
}

fn fibo_total<M: measurement::Measurement>(name: &str, iterations: u64, c: &mut BenchmarkGroup<M>) {
    let limit = 10_000_000_000;
    let lang_vesta = Lang::<pasta_curves::Fq, Coproc<pasta_curves::Fq>>::new();
    let reduction_count = DEFAULT_REDUCTION_COUNT;

    // use cached public params
    let pp = fcomm::public_params(reduction_count).unwrap();

    c.bench_with_input(
        BenchmarkId::new(name.to_string(), iterations),
        &(iterations),
        |b, iterations| {
            let mut store = Store::default();
            let env = empty_sym_env(&store);
            let ptr = fib::<pasta_curves::Fq>(&mut store, black_box(*iterations));
            let prover = NovaProver::new(reduction_count, lang_vesta.clone());

            b.iter(|| {
                let result = prover
                    .evaluate_and_prove(&pp, ptr, env, &mut store, limit, &lang_vesta)
                    .unwrap();
                black_box(result);
            })
        },
    );
}

fn fibo_eval<M: measurement::Measurement>(name: &str, iterations: u64, c: &mut BenchmarkGroup<M>) {
    let limit = 10_000_000_000;
    let lang_pallas = Lang::<pasta_curves::Fp, Coproc<pasta_curves::Fp>>::new();

    c.bench_with_input(
        BenchmarkId::new(name.to_string(), iterations),
        &(iterations),
        |b, iterations| {
            let mut store = Store::default();
            let ptr = fib::<pasta_curves::Fp>(&mut store, black_box(*iterations));
            b.iter(|| {
                let result =
                    Evaluator::new(ptr, empty_sym_env(&store), &mut store, limit, &lang_pallas)
                        .eval();
                black_box(result)
            });
        },
    );
}

fn fibo_prove<M: measurement::Measurement>(name: &str, iterations: u64, c: &mut BenchmarkGroup<M>) {
    let limit = 10_000_000_000;
    let lang_vesta = Lang::<pasta_curves::Fq, Coproc<pasta_curves::Fq>>::new();
    let reduction_count = DEFAULT_REDUCTION_COUNT;
    let pp = fcomm::public_params(reduction_count).unwrap();

    c.bench_with_input(
        BenchmarkId::new(name.to_string(), iterations),
        &(iterations),
        |b, iterations| {
            let mut store = Store::default();
            let env = empty_sym_env(&store);
            let ptr = fib::<pasta_curves::Fq>(&mut store, black_box(*iterations));
            let prover = NovaProver::new(reduction_count, lang_vesta.clone());

            let frames = prover
                .get_evaluation_frames(ptr, env, &mut store, limit, &lang_vesta)
                .unwrap();

            b.iter(|| {
                let result = prover
                    .prove(&pp, frames.clone(), &mut store, &lang_vesta)
                    .unwrap();
                black_box(result);
            })
        },
    );
}

fn fibonacci_eval(c: &mut Criterion) {
    static BATCH_SIZES: [u64; 2] = [100, 1000];
    let mut group: BenchmarkGroup<_> = c.benchmark_group("Evaluate");
    for size in BATCH_SIZES.iter() {
        fibo_eval("Fibonacci", *size, &mut group);
    }
}

fn fibonacci_prove(c: &mut Criterion) {
    static BATCH_SIZES: [u64; 2] = [100, 1000];
    let mut group: BenchmarkGroup<_> = c.benchmark_group("Prove");
    group.sampling_mode(SamplingMode::Flat); // This can take a *while*
    group.sample_size(10);

    for size in BATCH_SIZES.iter() {
        fibo_prove("Fibonacci", *size, &mut group);
    }
}

fn fibonacci_total(c: &mut Criterion) {
    static BATCH_SIZES: [u64; 2] = [100, 1000];
    let mut group: BenchmarkGroup<_> = c.benchmark_group("Total");
    group.sampling_mode(SamplingMode::Flat); // This can take a *while*
    group.sample_size(10);

    for size in BATCH_SIZES.iter() {
        fibo_total("Fibonacci", *size, &mut group);
    }
}

cfg_if::cfg_if! {
    if #[cfg(feature = "flamegraph")] {
        criterion_group! {
            name = benches;
            config = Criterion::default()
            .measurement_time(Duration::from_secs(120))
            .sample_size(10)
            .with_profiler(pprof::criterion::PProfProfiler::new(100, pprof::criterion::Output::Flamegraph(None)));
            targets =
            fibonacci_eval,
            fibonacci_prove,
            fibonacci_total,
        }
    } else {
        criterion_group! {
            name = benches;
            config = Criterion::default()
            .measurement_time(Duration::from_secs(120))
            .sample_size(10);
            targets =
            fibonacci_eval,
            fibonacci_prove,
            fibonacci_total,
        }
    }
}

criterion_main!(benches);
