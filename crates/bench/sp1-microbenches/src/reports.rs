use crate::fit::LinearFit;
use crate::BenchResult;

pub struct ReportContext<'a> {
    pub sp1_version: &'a str,
    pub git_commit: &'a str,
    pub date: &'a str,
}

/// Everything a bench's `run` returns for the dispatcher to handle.
pub struct BenchOutput {
    /// Rendered report markdown.
    pub markdown: String,
    /// Filename to use when the user didn't pass `--out`. Joined to the crate's `reports/` dir.
    pub default_filename: String,
    /// Short stdout summary, printed by the dispatcher after writing the file.
    pub summary: String,
}

/// Bench-specific content injected into the otherwise-shared report. Each bench implements this
/// on a zero-sized struct in its `cmd/<name>.rs` and passes it to `write_markdown`.
pub trait ReportContent {
    /// Header text after the leading `# `. E.g. "SHA-256 via MeteredHasher + …".
    fn algorithm(&self) -> &str;

    /// Markdown paragraph for the `## Scope` section.
    fn scope(&self) -> &str;

    /// Bullet-list markdown for the body of `## Suggested ZK gas constants`, given the fit.
    fn suggested_constants(&self, gas_fit: &LinearFit) -> String;

    /// Glossary entries unique to this bench, appended after the shared core entries.
    /// Each entry is `(term, definition_markdown)`.
    fn extra_glossary(&self) -> &'static [(&'static str, &'static str)];
}

const CORE_GLOSSARY: &[(&str, &str)] = &[
    (
        "prover gas",
        "SP1's `ExecutionReport.gas()` value. A predictor of GPU proving time computed from the execution trace's per-shard padded heights, normalized so it's of similar magnitude to RISC-V cycles. *Not* the same as the rollup's application gas in `constants.toml`; mapping prover gas to application gas requires picking a global scaling factor once all primitives are calibrated.",
    ),
    (
        "RISC-V cycles",
        "Count of RISC-V instructions executed by the SP1 zkVM. Easy to reason about but a worse predictor of proving cost than prover gas because precompile cycles (e.g. SHA-256 ecall) are far cheaper to prove per-cycle than ordinary cycles. Reported as a sanity check.",
    ),
    (
        "iter / iters",
        "Loop iterations of the inner benchmark loop. We run many iterations per execution to amortize the per-program startup cost of the SP1 executor over the operation under measurement. `gas/iter = prover_gas_total / iters`.",
    ),
    (
        "region cycles",
        "Cycles measured inside the labeled `cycle-tracker-report-{start,end}` markers in the guest. Captures only the inner work loop, excluding program startup and input reading. Available because SP1 exposes per-region cycles via `ExecutionReport.cycle_tracker`. No equivalent exists for prover gas, which is whole-execution-only — hence the isolated microbench design.",
    ),
    (
        "total cycles",
        "Whole-program cycle count (`ExecutionReport.total_instruction_count()`). Includes startup, input reading, loop overhead, commit, and the work itself. Always larger than `region cycles`.",
    ),
    (
        "bias",
        "The intercept of the linear fit, in *prover gas per call*. The fixed cost paid every time the operation runs, regardless of input size — precompile syscall setup, padding/framing, finalization, wrapper overhead. The specific gas constant in `constants.toml` it maps to is named in the bench's *Suggested ZK gas constants* section.",
    ),
    (
        "per_byte",
        "The slope of the linear fit, in *prover gas per byte of input*. The marginal cost of one additional input byte. The specific gas constant in `constants.toml` it maps to is named in the bench's *Suggested ZK gas constants* section.",
    ),
    (
        "R²",
        "Coefficient of determination for the linear fit. 1.0 means the linear model perfectly explains the variance in the measurements; 0.0 means no relationship. Anything below ~0.99 across an input-size sweep means the operation is not well-modeled as `bias + per_byte × size` and needs a richer cost function.",
    ),
    (
        "max residual",
        "The largest single-point deviation between measured cost and the linear-fit prediction, in the fit's units. With R² near 1 this is the worst-case error of using the linear model — useful for spotting step-function structure (e.g. SHA-256 processes 64-byte blocks, so residuals oscillate at small sizes and shrink as inputs grow).",
    ),
    (
        "`UnlimitedGasMeter`",
        "Stateless gas meter (`PhantomData<S>`) whose `charge_gas` and `charge_linear_gas` are no-ops via the `GasMeter` trait's default impls. Used so the metering wrapper compiles in but inlines to zero work — we measure the primitive itself, not the meter arithmetic.",
    ),
];

pub fn render_markdown(
    ctx: &ReportContext,
    content: &impl ReportContent,
    results: &[BenchResult],
    gas_fit: &LinearFit,
    cycles_fit: &LinearFit,
) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "# {} prover-gas microbench\n\n",
        content.algorithm()
    ));
    s.push_str(&format!("- Date: {}\n", ctx.date));
    s.push_str(&format!("- SP1 SDK: {}\n", ctx.sp1_version));
    s.push_str(&format!("- Git commit: {}\n", ctx.git_commit));
    s.push_str("- Method: `ProverClient::execute()` (no proving).\n");
    s.push_str("- Iterations per run: see table.\n");
    s.push_str(
        "- Term definitions: see [Appendix: Glossary](#appendix-glossary) at the bottom.\n\n",
    );

    s.push_str("## Raw measurements\n\n");
    s.push_str("| bytes | iters | prover gas (total) | gas/iter | total cycles | region cycles | region cycles/iter |\n");
    s.push_str("|------:|------:|------------------:|--------:|-------------:|--------------:|------------------:|\n");
    for r in results {
        s.push_str(&format!(
            "| {} | {} | {} | {:.2} | {} | {} | {:.2} |\n",
            r.input_size,
            r.iterations,
            r.prover_gas,
            r.per_iter_prover_gas(),
            r.total_cycles,
            r.region_cycles,
            r.per_iter_region_cycles(),
        ));
    }

    s.push_str("\n## Linear fit: prover gas per call\n\n");
    s.push_str("Model: `gas_per_call = bias + per_byte * input_size`.\n\n");
    s.push_str(&format!("- bias (gas / call): **{:.2}**\n", gas_fit.bias));
    s.push_str(&format!(
        "- per_byte (gas / byte): **{:.4}**\n",
        gas_fit.per_byte
    ));
    s.push_str(&format!("- R²: {:.6}\n", gas_fit.r_squared));
    s.push_str(&format!(
        "- max residual: {:.2} gas\n\n",
        gas_fit.max_residual
    ));

    s.push_str("## Linear fit: RISC-V cycles per call (sanity check)\n\n");
    s.push_str(&format!("- bias (cycles / call): {:.2}\n", cycles_fit.bias));
    s.push_str(&format!(
        "- per_byte (cycles / byte): {:.4}\n",
        cycles_fit.per_byte
    ));
    s.push_str(&format!("- R²: {:.6}\n", cycles_fit.r_squared));
    s.push_str(&format!(
        "- max residual: {:.2} cycles\n\n",
        cycles_fit.max_residual
    ));

    s.push_str("## Scope\n\n");
    s.push_str(content.scope());
    s.push_str("\n\n");

    s.push_str("## Suggested ZK gas constants\n\n");
    s.push_str("Raw values (rounded). Apply a global scaling factor when wiring into `constants.toml` so that all metered primitives share a coherent unit.\n\n");
    s.push_str(&content.suggested_constants(gas_fit));
    s.push_str("\n");

    s.push_str("\n## Appendix: Glossary\n\n");
    for (term, definition) in CORE_GLOSSARY {
        s.push_str(&format!("**{}** — {}\n\n", term, definition));
    }
    for (term, definition) in content.extra_glossary() {
        s.push_str(&format!("**{}** — {}\n\n", term, definition));
    }

    s
}
