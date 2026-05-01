use std::env;
use std::fs;
use std::path::Path;

const AUDIO_OUTPUT_FILTER_TAPS: [i32; 8] = [49, 11, -3, 18, -15, 3, 7, -6];
const AUDIO_OUTPUT_FILTER_DEN: i32 = 64;
const AUDIO_OUTPUT_POST_GAIN_NUM: i32 = 127;
const AUDIO_OUTPUT_POST_GAIN_DEN: i32 = 128;
const AUDIO_OUTPUT_COMPRESS_DEN: i32 = 128;
const AUDIO_OUTPUT_POST_FILTER_DEN: i32 = 128;

const SEARCH_DEAD_DELTAS: [i32; 9] = [-4, -3, -2, -1, 0, 1, 2, 3, 4];
const SEARCH_THR_DELTAS: [i32; 7] = [-80, -40, -20, 0, 20, 40, 80];
const SEARCH_CNUM_DELTAS: [i32; 9] = [-4, -3, -2, -1, 0, 1, 2, 3, 4];
const SEARCH_BIAS_DELTAS: [i32; 9] = [-4, -3, -2, -1, 0, 1, 2, 3, 4];
const SEARCH_POST_DELTAS: [i32; 9] = [-4, -3, -2, -1, 0, 1, 2, 3, 4];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AudioOutputParams {
    deadzone: i32,
    compress_threshold: i32,
    compress_num: i32,
    positive_bias: i32,
    negative_bias: i32,
    post_filter_cur: i32,
    post_filter_prev: i32,
    post_filter_prev2: i32,
    post_filter_bias: i32,
}

impl Default for AudioOutputParams {
    fn default() -> Self {
        // Keep these in sync with the late-stage audio constants in `src/lib.rs`.
        Self {
            deadzone: 5,
            compress_threshold: 2_380,
            compress_num: 127,
            positive_bias: 74,
            negative_bias: 67,
            post_filter_cur: 129,
            post_filter_prev: -1,
            post_filter_prev2: 0,
            post_filter_bias: -1,
        }
    }
}

#[derive(Clone)]
struct Dataset {
    label: String,
    prefilter: Vec<i16>,
    reference: Vec<i16>,
    reference_first_nonzero_pair: usize,
}

#[derive(Clone)]
struct CandidateScore {
    total_rmse: f64,
    rmses: Vec<f64>,
    first_nonzero_pairs: Vec<usize>,
}

#[derive(Clone, Copy)]
enum ParamKind {
    Deadzone,
    CompressThreshold,
    CompressNum,
    PositiveBias,
    NegativeBias,
    PostFilterCur,
    PostFilterPrev,
    PostFilterPrev2,
    PostFilterBias,
}

impl ParamKind {
    const ALL: [ParamKind; 9] = [
        ParamKind::Deadzone,
        ParamKind::CompressThreshold,
        ParamKind::CompressNum,
        ParamKind::PositiveBias,
        ParamKind::NegativeBias,
        ParamKind::PostFilterCur,
        ParamKind::PostFilterPrev,
        ParamKind::PostFilterPrev2,
        ParamKind::PostFilterBias,
    ];

    fn deltas(self) -> &'static [i32] {
        match self {
            ParamKind::Deadzone => &SEARCH_DEAD_DELTAS,
            ParamKind::CompressThreshold => &SEARCH_THR_DELTAS,
            ParamKind::CompressNum => &SEARCH_CNUM_DELTAS,
            ParamKind::PositiveBias | ParamKind::NegativeBias => &SEARCH_BIAS_DELTAS,
            ParamKind::PostFilterCur
            | ParamKind::PostFilterPrev
            | ParamKind::PostFilterPrev2
            | ParamKind::PostFilterBias => {
                &SEARCH_POST_DELTAS
            }
        }
    }

    fn apply(self, params: &mut AudioOutputParams, delta: i32) {
        match self {
            ParamKind::Deadzone => params.deadzone = (params.deadzone + delta).max(0),
            ParamKind::CompressThreshold => {
                params.compress_threshold = (params.compress_threshold + delta).max(0)
            }
            ParamKind::CompressNum => params.compress_num = (params.compress_num + delta).clamp(120, 128),
            ParamKind::PositiveBias => params.positive_bias += delta,
            ParamKind::NegativeBias => params.negative_bias += delta,
            ParamKind::PostFilterCur => params.post_filter_cur = (params.post_filter_cur + delta).clamp(120, 136),
            ParamKind::PostFilterPrev => params.post_filter_prev = (params.post_filter_prev + delta).clamp(-8, 8),
            ParamKind::PostFilterPrev2 => params.post_filter_prev2 = (params.post_filter_prev2 + delta).clamp(-8, 8),
            ParamKind::PostFilterBias => params.post_filter_bias += delta,
        }
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let mut max_first_regression = 0.01f64;
    let mut require_first_nonzero_match = false;
    let mut start_params = AudioOutputParams::default();
    let mut positional = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--max-first-regression" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing regression value".to_string())?;
                max_first_regression = value
                    .parse::<f64>()
                    .map_err(|_| "regression value must be a number".to_string())?;
                if max_first_regression < 0.0 {
                    return Err("regression value must be nonnegative".to_string());
                }
            }
            "--require-first-nonzero-match" => require_first_nonzero_match = true,
            "--start-deadzone" => {
                start_params.deadzone = parse_i32_arg(args.next(), "missing deadzone value")?.max(0);
            }
            "--start-threshold" => {
                start_params.compress_threshold =
                    parse_i32_arg(args.next(), "missing threshold value")?.max(0);
            }
            "--start-compress-num" => {
                start_params.compress_num =
                    parse_i32_arg(args.next(), "missing compress numerator value")?.clamp(120, 128);
            }
            "--start-positive-bias" => {
                start_params.positive_bias =
                    parse_i32_arg(args.next(), "missing positive bias value")?;
            }
            "--start-negative-bias" => {
                start_params.negative_bias =
                    parse_i32_arg(args.next(), "missing negative bias value")?;
            }
            "--start-post-filter-cur" => {
                start_params.post_filter_cur =
                    parse_i32_arg(args.next(), "missing post-filter cur value")?.clamp(120, 136);
            }
            "--start-post-filter-prev" => {
                start_params.post_filter_prev =
                    parse_i32_arg(args.next(), "missing post-filter prev value")?.clamp(-8, 8);
            }
            "--start-post-filter-prev2" => {
                start_params.post_filter_prev2 =
                    parse_i32_arg(args.next(), "missing post-filter prev2 value")?.clamp(-8, 8);
            }
            "--start-post-filter-bias" => {
                start_params.post_filter_bias =
                    parse_i32_arg(args.next(), "missing post-filter bias value")?;
            }
            _ => positional.push(arg),
        }
    }

    if positional.len() < 2 || positional.len() % 2 != 0 {
        return Err(
            "usage: tune_audio [--max-first-regression value] <prefilter.wav> <reference.wav> [<prefilter.wav> <reference.wav> ...]"
                .to_string(),
        );
    }

    let mut datasets = Vec::new();
    for pair in positional.chunks_exact(2) {
        let prefilter = read_wav_i16(Path::new(&pair[0]))?;
        let reference = read_wav_i16(Path::new(&pair[1]))?;
        if prefilter.sample_rate != reference.sample_rate {
            return Err(format!(
                "sample-rate mismatch for {} vs {}",
                pair[0], pair[1]
            ));
        }
        let prefilter_channel = wav_first_channel(&prefilter.samples, prefilter.channels);
        let reference_channel = wav_first_channel(&reference.samples, reference.channels);
        let len = prefilter_channel.len().min(reference_channel.len());
        datasets.push(Dataset {
            label: format!("{} -> {}", pair[0], pair[1]),
            prefilter: prefilter_channel[..len].to_vec(),
            reference: reference_channel[..len].to_vec(),
            reference_first_nonzero_pair: first_nonzero_pair(&reference_channel[..len]),
        });
    }

    let baseline_params = start_params;
    let baseline = score_candidate(&datasets, baseline_params);
    print_score("baseline", baseline_params, &datasets, &baseline);

    let best_params = search(
        &datasets,
        baseline_params,
        &baseline,
        max_first_regression,
        require_first_nonzero_match,
    );
    let best_score = score_candidate(&datasets, best_params);
    print_score("best", best_params, &datasets, &best_score);

    Ok(())
}

fn parse_i32_arg(value: Option<String>, missing: &str) -> Result<i32, String> {
    value
        .ok_or_else(|| missing.to_string())?
        .parse::<i32>()
        .map_err(|_| missing.replace("missing", "invalid"))
}

fn search(
    datasets: &[Dataset],
    start: AudioOutputParams,
    baseline: &CandidateScore,
    max_first_regression: f64,
    require_first_nonzero_match: bool,
) -> AudioOutputParams {
    let mut best_params = start;
    let mut best_score = baseline.clone();

    loop {
        let mut improved = false;
        let mut round_best_params = best_params;
        let mut round_best_score = best_score.clone();

        for (i, first) in ParamKind::ALL.iter().enumerate() {
            for &first_delta in first.deltas() {
                if first_delta == 0 {
                    continue;
                }
                let mut candidate = best_params;
                first.apply(&mut candidate, first_delta);
                consider_candidate(
                    datasets,
                    baseline,
                    max_first_regression,
                    require_first_nonzero_match,
                    candidate,
                    &mut round_best_params,
                    &mut round_best_score,
                );
            }
            for second in ParamKind::ALL.iter().skip(i + 1) {
                for &first_delta in first.deltas() {
                    for &second_delta in second.deltas() {
                        if first_delta == 0 && second_delta == 0 {
                            continue;
                        }
                        let mut candidate = best_params;
                        first.apply(&mut candidate, first_delta);
                        second.apply(&mut candidate, second_delta);
                        consider_candidate(
                            datasets,
                            baseline,
                            max_first_regression,
                            require_first_nonzero_match,
                            candidate,
                            &mut round_best_params,
                            &mut round_best_score,
                        );
                    }
                }
            }
        }

        if round_best_score.total_rmse + 1e-9 < best_score.total_rmse {
            improved = true;
            best_params = round_best_params;
            best_score = round_best_score;
            print_score("improved", best_params, datasets, &best_score);
        }

        if !improved {
            return best_params;
        }
    }
}

fn consider_candidate(
    datasets: &[Dataset],
    baseline: &CandidateScore,
    max_first_regression: f64,
    require_first_nonzero_match: bool,
    params: AudioOutputParams,
    best_params: &mut AudioOutputParams,
    best_score: &mut CandidateScore,
) {
    let score = score_candidate(datasets, params);
    if score.rmses[0] > baseline.rmses[0] + max_first_regression {
        return;
    }
    if require_first_nonzero_match
        && score
            .first_nonzero_pairs
            .iter()
            .zip(datasets.iter())
            .any(|(pair, dataset)| *pair != dataset.reference_first_nonzero_pair)
    {
        return;
    }
    if score.total_rmse + 1e-9 < best_score.total_rmse {
        *best_params = params;
        *best_score = score;
    }
}

fn score_candidate(datasets: &[Dataset], params: AudioOutputParams) -> CandidateScore {
    let mut total = 0.0;
    let mut rmses = Vec::with_capacity(datasets.len());
    let mut first_nonzero_pairs = Vec::with_capacity(datasets.len());
    for dataset in datasets {
        let (rmse, first_nonzero_pair) = evaluate_dataset(dataset, params);
        total += rmse;
        rmses.push(rmse);
        first_nonzero_pairs.push(first_nonzero_pair);
    }
    CandidateScore {
        total_rmse: total,
        rmses,
        first_nonzero_pairs,
    }
}

fn evaluate_dataset(dataset: &Dataset, params: AudioOutputParams) -> (f64, usize) {
    let mut filter_history = [0i32; AUDIO_OUTPUT_FILTER_TAPS.len() - 1];
    let mut post_history = 0i16;
    let mut post_history2 = 0i16;
    let mut error_sum = 0u128;
    let mut first_nonzero_pair = dataset.reference.len();

    for (idx, (&scaled, &reference)) in dataset
        .prefilter
        .iter()
        .zip(dataset.reference.iter())
        .enumerate()
    {
        let output = filter_audio_sample(
            i32::from(scaled),
            &mut filter_history,
            &mut post_history,
            &mut post_history2,
            params,
        );
        if output != 0 && first_nonzero_pair == dataset.reference.len() {
            first_nonzero_pair = idx;
        }
        let error = i64::from(output) - i64::from(reference);
        error_sum += (error * error) as u128;
    }

    (
        (error_sum as f64 / dataset.reference.len() as f64).sqrt(),
        first_nonzero_pair,
    )
}

fn filter_audio_sample(
    scaled: i32,
    filter_history: &mut [i32; AUDIO_OUTPUT_FILTER_TAPS.len() - 1],
    post_history: &mut i16,
    post_history2: &mut i16,
    params: AudioOutputParams,
) -> i16 {
    let mut accum = AUDIO_OUTPUT_FILTER_TAPS[0] * scaled;
    for (tap, history) in AUDIO_OUTPUT_FILTER_TAPS[1..].iter().zip(filter_history.iter()) {
        accum += *tap * *history;
    }
    let filtered = round_divide(accum, AUDIO_OUTPUT_FILTER_DEN);
    let history_len = filter_history.len();
    filter_history.copy_within(0..history_len - 1, 1);
    filter_history[0] = scaled;

    let clamped = filtered.clamp(i16::MIN as i32, i16::MAX as i32);
    if clamped.abs() <= params.deadzone {
        return 0;
    }

    let gained = round_divide(clamped * AUDIO_OUTPUT_POST_GAIN_NUM, AUDIO_OUTPUT_POST_GAIN_DEN)
        .clamp(i16::MIN as i32, i16::MAX as i32);
    let compressed = if gained.abs() > params.compress_threshold {
        let sign = gained.signum();
        let above = gained.abs() - params.compress_threshold;
        sign * (params.compress_threshold
            + round_divide(above * params.compress_num, AUDIO_OUTPUT_COMPRESS_DEN))
    } else {
        gained
    };
    let biased = if compressed > 0 {
        compressed + params.positive_bias
    } else {
        compressed + params.negative_bias
    };
    let post_filtered = round_divide(
        biased * params.post_filter_cur
            + i32::from(*post_history) * params.post_filter_prev
            + i32::from(*post_history2) * params.post_filter_prev2,
        AUDIO_OUTPUT_POST_FILTER_DEN,
    )
    .clamp(i16::MIN as i32, i16::MAX as i32);
    *post_history2 = *post_history;
    *post_history = biased.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    if post_filtered == 0 {
        0
    } else {
        (post_filtered + params.post_filter_bias)
            .clamp(i16::MIN as i32, i16::MAX as i32) as i16
    }
}

fn print_score(label: &str, params: AudioOutputParams, datasets: &[Dataset], score: &CandidateScore) {
    println!(
        "{label} dead={} thr={} cnum={} pos_bias={} neg_bias={} cur={} prev={} prev2={} post_bias={} total_rmse={:.6}",
        params.deadzone,
        params.compress_threshold,
        params.compress_num,
        params.positive_bias,
        params.negative_bias,
        params.post_filter_cur,
        params.post_filter_prev,
        params.post_filter_prev2,
        params.post_filter_bias,
        score.total_rmse
    );
    for ((dataset, rmse), first_nonzero_pair) in datasets
        .iter()
        .zip(score.rmses.iter())
        .zip(score.first_nonzero_pairs.iter())
    {
        println!(
            "  rmse {:.6} first_nonzero={} ref={} {}",
            rmse, first_nonzero_pair, dataset.reference_first_nonzero_pair, dataset.label
        );
    }
}

struct WavData {
    sample_rate: u32,
    channels: u16,
    samples: Vec<i16>,
}

fn read_wav_i16(path: &Path) -> Result<WavData, String> {
    let bytes = fs::read(path).map_err(|e| format!("failed to read wav {}: {e}", path.display()))?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(format!("wav {} must be RIFF/WAVE", path.display()));
    }

    let mut offset = 12usize;
    let mut channels = None;
    let mut sample_rate = None;
    let mut bits_per_sample = None;
    let mut audio_format = None;
    let mut data = None;

    while offset + 8 <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        offset += 8;
        if offset + chunk_size > bytes.len() {
            return Err(format!("wav chunk in {} extends past end of file", path.display()));
        }
        let chunk = &bytes[offset..offset + chunk_size];
        match chunk_id {
            b"fmt " => {
                if chunk.len() < 16 {
                    return Err(format!("wav fmt chunk in {} is too small", path.display()));
                }
                audio_format = Some(u16::from_le_bytes(chunk[0..2].try_into().unwrap()));
                channels = Some(u16::from_le_bytes(chunk[2..4].try_into().unwrap()));
                sample_rate = Some(u32::from_le_bytes(chunk[4..8].try_into().unwrap()));
                bits_per_sample = Some(u16::from_le_bytes(chunk[14..16].try_into().unwrap()));
            }
            b"data" => data = Some(chunk.to_vec()),
            _ => {}
        }
        offset += chunk_size + (chunk_size & 1);
    }

    if audio_format != Some(1) {
        return Err(format!("wav {} must be PCM", path.display()));
    }
    let channels = channels.ok_or_else(|| format!("wav {} is missing channel count", path.display()))?;
    let sample_rate = sample_rate.ok_or_else(|| format!("wav {} is missing sample rate", path.display()))?;
    if bits_per_sample != Some(16) {
        return Err(format!("wav {} must use 16-bit samples", path.display()));
    }
    let data = data.ok_or_else(|| format!("wav {} is missing a data chunk", path.display()))?;
    if data.len() % 2 != 0 {
        return Err(format!("wav {} data length must be even", path.display()));
    }

    let mut samples = Vec::with_capacity(data.len() / 2);
    for chunk in data.chunks_exact(2) {
        samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }

    Ok(WavData {
        sample_rate,
        channels,
        samples,
    })
}

fn wav_first_channel(samples: &[i16], channels: u16) -> Vec<i16> {
    let stride = channels as usize;
    if stride <= 1 {
        return samples.to_vec();
    }

    if samples.len() % stride != 0 {
        return samples.to_vec();
    }

    let frame_count = samples.len() / stride;
    let mut first = Vec::with_capacity(frame_count);
    for frame in samples.chunks_exact(stride) {
        first.push(frame[0]);
    }
    first
}

fn first_nonzero_pair(samples: &[i16]) -> usize {
    samples
        .iter()
        .position(|sample| *sample != 0)
        .unwrap_or(samples.len())
}

fn round_divide(value: i32, denominator: i32) -> i32 {
    if value >= 0 {
        (value + denominator / 2) / denominator
    } else {
        -((-value + denominator / 2) / denominator)
    }
}
