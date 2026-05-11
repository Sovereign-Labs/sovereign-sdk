//! Ordinary least squares fit of the cost model `cost = bias + per_byte * input_size`.

use crate::BenchResult;

#[derive(Debug, Clone)]
pub struct LinearFit {
    /// Intercept of the fitted line: estimated per-call fixed overhead (cost at input_size = 0).
    pub bias: f64,
    /// Slope of the fitted line: estimated marginal cost per byte of input.
    pub per_byte: f64,
    /// Coefficient of determination; 1.0 means the line explains the data perfectly, 0.0 means it explains none of the variation.
    pub r_squared: f64,
    /// Largest absolute deviation between any measured point and the fitted line, in the cost's units.
    pub max_residual: f64,
}

/// Fit `prover_gas_per_call = bias + per_byte * input_size` over the bench results.
pub fn fit_prover_gas_per_byte(results: &[BenchResult]) -> anyhow::Result<LinearFit> {
    let input_sizes: Vec<f64> = results.iter().map(|r| r.input_size as f64).collect();
    let prover_gas: Vec<f64> = results.iter().map(|r| r.per_iter_prover_gas()).collect();
    fit_linear(&input_sizes, &prover_gas)
}

/// Fit `region_cycles_per_call = bias + per_byte * input_size` over the bench results.
pub fn fit_region_cycles_per_byte(results: &[BenchResult]) -> anyhow::Result<LinearFit> {
    let input_sizes: Vec<f64> = results.iter().map(|r| r.input_size as f64).collect();
    let region_cycles: Vec<f64> = results.iter().map(|r| r.per_iter_region_cycles()).collect();
    fit_linear(&input_sizes, &region_cycles)
}

/// Given a list of `(input_size, measured_cost)` points, find the straight line that best
/// describes their relationship.
///
/// We use this to extract the two numbers our gas model needs: a per-call fixed overhead
/// (the line's intercept, surfaced as `bias`) and a per-byte marginal cost (the line's slope,
/// surfaced as `per_byte`). Together they let us charge gas with one formula:
/// `cost = bias + per_byte × size`.
///
/// "Best" means ordinary least squares: pick the line that minimises the sum of the squared
/// vertical distances from each data point to the line.
///
/// Implemented ourselves rather than pulled from `linregress` or `linfa`: single-regressor OLS
/// is a few lines of closed-form arithmetic, we don't need confidence intervals or p-values,
/// and avoiding the dependency keeps `ndarray` and friends out of the build graph.
fn fit_linear(inputs: &[f64], measurements: &[f64]) -> anyhow::Result<LinearFit> {
    if inputs.len() < 2 {
        anyhow::bail!(
            "need at least 2 data points to fit a line, got {}",
            inputs.len()
        );
    }

    let sample_count = inputs.len() as f64;
    let sum_x: f64 = inputs.iter().sum();
    let sum_y: f64 = measurements.iter().sum();
    let sum_x_squared: f64 = inputs.iter().map(|x| x * x).sum();
    let sum_xy: f64 = inputs.iter().zip(measurements).map(|(x, y)| x * y).sum();

    let slope_denominator = sample_count * sum_x_squared - sum_x * sum_x;
    if slope_denominator.abs() < f64::EPSILON {
        anyhow::bail!("cannot fit: all input values are identical");
    }
    let slope = (sample_count * sum_xy - sum_x * sum_y) / slope_denominator;
    let intercept = (sum_y - slope * sum_x) / sample_count;

    let mean_measurement = sum_y / sample_count;
    let mut total_sum_of_squares = 0.0;
    let mut residual_sum_of_squares = 0.0;
    let mut max_residual = 0.0_f64;
    for (&x, &measured) in inputs.iter().zip(measurements) {
        let predicted = intercept + slope * x;
        let signed_residual = measured - predicted;
        max_residual = max_residual.max(signed_residual.abs());
        residual_sum_of_squares += signed_residual.powi(2);
        total_sum_of_squares += (measured - mean_measurement).powi(2);
    }
    let r_squared = if total_sum_of_squares < f64::EPSILON {
        1.0
    } else {
        1.0 - residual_sum_of_squares / total_sum_of_squares
    };

    Ok(LinearFit {
        bias: intercept,
        per_byte: slope,
        r_squared,
        max_residual,
    })
}
