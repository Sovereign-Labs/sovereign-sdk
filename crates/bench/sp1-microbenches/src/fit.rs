use crate::BenchResult;

/// Result of an OLS fit `y = bias + per_byte * size`.
#[derive(Debug, Clone)]
pub struct LinearFit {
    pub bias: f64,
    pub per_byte: f64,
    pub r_squared: f64,
    pub max_residual: f64,
}

/// Fit per-iteration prover gas as a linear function of input size.
///
/// Drops `byte_len == 0` baseline points before fitting since they only constrain the bias term;
/// the bias value is taken straight from the smallest-size point's residual instead.
pub fn fit_prover_gas_per_byte(results: &[BenchResult]) -> anyhow::Result<LinearFit> {
    let xs: Vec<f64> = results.iter().map(|r| r.byte_len as f64).collect();
    let ys: Vec<f64> = results.iter().map(|r| r.per_iter_prover_gas()).collect();
    fit_ols(&xs, &ys)
}

/// Same fit applied to the hash-loop region cycle count.
pub fn fit_hash_cycles_per_byte(results: &[BenchResult]) -> anyhow::Result<LinearFit> {
    let xs: Vec<f64> = results.iter().map(|r| r.byte_len as f64).collect();
    let ys: Vec<f64> = results.iter().map(|r| r.per_iter_hash_cycles()).collect();
    fit_ols(&xs, &ys)
}

fn fit_ols(xs: &[f64], ys: &[f64]) -> anyhow::Result<LinearFit> {
    if xs.len() < 2 {
        anyhow::bail!("need at least 2 data points to fit a line, got {}", xs.len());
    }
    let n = xs.len() as f64;
    let sx: f64 = xs.iter().sum();
    let sy: f64 = ys.iter().sum();
    let sxx: f64 = xs.iter().map(|x| x * x).sum();
    let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| x * y).sum();

    let denom = n * sxx - sx * sx;
    if denom.abs() < f64::EPSILON {
        anyhow::bail!("cannot fit: all x values are identical");
    }
    let per_byte = (n * sxy - sx * sy) / denom;
    let bias = (sy - per_byte * sx) / n;

    let mean_y = sy / n;
    let mut ss_tot = 0.0;
    let mut ss_res = 0.0;
    let mut max_residual = 0.0_f64;
    for (x, y) in xs.iter().zip(ys) {
        let predicted = bias + per_byte * x;
        let residual = (y - predicted).abs();
        max_residual = max_residual.max(residual);
        ss_res += (y - predicted).powi(2);
        ss_tot += (y - mean_y).powi(2);
    }
    let r_squared = if ss_tot < f64::EPSILON {
        1.0
    } else {
        1.0 - ss_res / ss_tot
    };

    Ok(LinearFit {
        bias,
        per_byte,
        r_squared,
        max_residual,
    })
}
