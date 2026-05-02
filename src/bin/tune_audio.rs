use std::env;
use std::fs;
use std::path::Path;

const AUDIO_OUTPUT_FILTER_TAPS: [i32; 8] = [49, 11, -3, 18, -15, 3, 7, -6];
const AUDIO_OUTPUT_FILTER_DEN: i32 = 64;
const AUDIO_OUTPUT_DELAY_PAIRS: usize = 165;
const AUDIO_OUTPUT_GAIN_NUM: i32 = 1;
const AUDIO_OUTPUT_GAIN_DEN: i32 = 4;
const AUDIO_OUTPUT_POST_GAIN_NUM: i32 = 127;
const AUDIO_OUTPUT_POST_GAIN_DEN: i32 = 128;
const AUDIO_OUTPUT_INPUT_FILTER_DEN: i32 = 128;
const AUDIO_OUTPUT_PREFILTER_GAIN_DEN: i32 = 128;
const AUDIO_OUTPUT_COMPRESS_DEN: i32 = 128;
const AUDIO_OUTPUT_POST_FILTER_DEN: i32 = 128;
const AUDIO_OUTPUT_FINAL_FILTER_TAPS: [i32; 4] = [128, 0, 0, -8];
const AUDIO_OUTPUT_FINAL_FILTER_DEN: i32 = 128;

const SEARCH_INPUT_CUR_DELTAS: [i32; 7] = [-8, -4, -2, 0, 2, 4, 8];
const SEARCH_INPUT_PREV_DELTAS: [i32; 9] = [-8, -4, -2, -1, 0, 1, 2, 4, 8];
const SEARCH_DEAD_DELTAS: [i32; 9] = [-4, -3, -2, -1, 0, 1, 2, 3, 4];
const SEARCH_THR_DELTAS: [i32; 7] = [-80, -40, -20, 0, 20, 40, 80];
const SEARCH_CNUM_DELTAS: [i32; 9] = [-4, -3, -2, -1, 0, 1, 2, 3, 4];
const SEARCH_PREFILTER_GAIN_DELTAS: [i32; 7] = [-8, -4, -2, 0, 2, 4, 8];
const SEARCH_BIAS_DELTAS: [i32; 9] = [-4, -3, -2, -1, 0, 1, 2, 3, 4];
const SEARCH_POST_DELTAS: [i32; 9] = [-4, -3, -2, -1, 0, 1, 2, 3, 4];
const SEARCH_HYST_DELTAS: [i32; 9] = [-16, -8, -4, -2, 0, 2, 4, 8, 16];
const SEARCH_FINAL_DELTAS: [i32; 5] = [-2, -1, 0, 1, 2];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AudioOutputParams {
    deadzone: i32,
    input_filter_cur: i32,
    input_filter_prev: i32,
    prefilter_gain_num: i32,
    compress_threshold_positive: i32,
    compress_threshold_negative: i32,
    compress_num_positive: i32,
    compress_num_negative: i32,
    positive_bias: i32,
    negative_bias: i32,
    post_filter_cur: i32,
    post_filter_prev: i32,
    post_filter_prev2: i32,
    post_filter_positive_bias: i32,
    post_filter_negative_bias: i32,
    sign_hysteresis: i32,
    final_filter_cur: i32,
    final_filter_prev: i32,
    final_filter_prev2: i32,
    final_filter_prev3: i32,
    final_nonzero_bias: i32,
}

impl Default for AudioOutputParams {
    fn default() -> Self {
        // Keep these in sync with the late-stage audio constants in `src/lib.rs`.
        Self {
            deadzone: 0,
            input_filter_cur: 136,
            input_filter_prev: -16,
            prefilter_gain_num: 136,
            compress_threshold_positive: 2_400,
            compress_threshold_negative: 2_080,
            compress_num_positive: 128,
            compress_num_negative: 128,
            positive_bias: 72,
            negative_bias: 70,
            post_filter_cur: 136,
            post_filter_prev: -8,
            post_filter_prev2: 6,
            post_filter_positive_bias: 10,
            post_filter_negative_bias: -14,
            sign_hysteresis: 28,
            final_filter_cur: 128,
            final_filter_prev: 0,
            final_filter_prev2: 0,
            final_filter_prev3: -8,
            final_nonzero_bias: 3,
        }
    }
}

#[derive(Clone)]
struct Dataset {
    label: String,
    prefilter: Vec<i16>,
    reference: Vec<i16>,
    reference_first_nonzero_pair: usize,
    reference_peak: i32,
}

#[derive(Clone)]
struct CandidateScore {
    total_rmse: f64,
    objective_total: f64,
    total_peak_overage: i32,
    rmses: Vec<f64>,
    first_nonzero_pairs: Vec<usize>,
    peaks: Vec<i32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ParamKind {
    Deadzone,
    InputFilterCur,
    InputFilterPrev,
    PrefilterGainNum,
    CompressThresholdPositive,
    CompressThresholdNegative,
    CompressNumPositive,
    CompressNumNegative,
    PositiveBias,
    NegativeBias,
    PostFilterCur,
    PostFilterPrev,
    PostFilterPrev2,
    PostFilterPositiveBias,
    PostFilterNegativeBias,
    SignHysteresis,
    FinalFilterCur,
    FinalFilterPrev,
    FinalFilterPrev2,
    FinalFilterPrev3,
    FinalNonzeroBias,
}

impl ParamKind {
    const ALL: [ParamKind; 21] = [
        ParamKind::Deadzone,
        ParamKind::InputFilterCur,
        ParamKind::InputFilterPrev,
        ParamKind::PrefilterGainNum,
        ParamKind::CompressThresholdPositive,
        ParamKind::CompressThresholdNegative,
        ParamKind::CompressNumPositive,
        ParamKind::CompressNumNegative,
        ParamKind::PositiveBias,
        ParamKind::NegativeBias,
        ParamKind::PostFilterCur,
        ParamKind::PostFilterPrev,
        ParamKind::PostFilterPrev2,
        ParamKind::PostFilterPositiveBias,
        ParamKind::PostFilterNegativeBias,
        ParamKind::SignHysteresis,
        ParamKind::FinalFilterCur,
        ParamKind::FinalFilterPrev,
        ParamKind::FinalFilterPrev2,
        ParamKind::FinalFilterPrev3,
        ParamKind::FinalNonzeroBias,
    ];

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "dead" => ParamKind::Deadzone,
            "icur" => ParamKind::InputFilterCur,
            "iprev" => ParamKind::InputFilterPrev,
            "pregain" => ParamKind::PrefilterGainNum,
            "pthr" => ParamKind::CompressThresholdPositive,
            "nthr" => ParamKind::CompressThresholdNegative,
            "pcnum" => ParamKind::CompressNumPositive,
            "ncnum" => ParamKind::CompressNumNegative,
            "pos_bias" => ParamKind::PositiveBias,
            "neg_bias" => ParamKind::NegativeBias,
            "cur" => ParamKind::PostFilterCur,
            "prev" => ParamKind::PostFilterPrev,
            "prev2" => ParamKind::PostFilterPrev2,
            "post_pos_bias" => ParamKind::PostFilterPositiveBias,
            "post_neg_bias" => ParamKind::PostFilterNegativeBias,
            "sign_hyst" => ParamKind::SignHysteresis,
            "fcur" => ParamKind::FinalFilterCur,
            "fprev" => ParamKind::FinalFilterPrev,
            "fprev2" => ParamKind::FinalFilterPrev2,
            "fprev3" => ParamKind::FinalFilterPrev3,
            "fnonzero" => ParamKind::FinalNonzeroBias,
            _ => return None,
        })
    }

    fn deltas(self) -> &'static [i32] {
        match self {
            ParamKind::Deadzone => &SEARCH_DEAD_DELTAS,
            ParamKind::InputFilterCur => &SEARCH_INPUT_CUR_DELTAS,
            ParamKind::InputFilterPrev => &SEARCH_INPUT_PREV_DELTAS,
            ParamKind::PrefilterGainNum => &SEARCH_PREFILTER_GAIN_DELTAS,
            ParamKind::CompressThresholdPositive | ParamKind::CompressThresholdNegative => &SEARCH_THR_DELTAS,
            ParamKind::CompressNumPositive | ParamKind::CompressNumNegative => &SEARCH_CNUM_DELTAS,
            ParamKind::PositiveBias | ParamKind::NegativeBias => &SEARCH_BIAS_DELTAS,
            ParamKind::FinalNonzeroBias => &SEARCH_BIAS_DELTAS,
            ParamKind::SignHysteresis => &SEARCH_HYST_DELTAS,
            ParamKind::FinalFilterCur
            | ParamKind::FinalFilterPrev
            | ParamKind::FinalFilterPrev2
            | ParamKind::FinalFilterPrev3 => &SEARCH_FINAL_DELTAS,
            ParamKind::PostFilterCur
            | ParamKind::PostFilterPrev
            | ParamKind::PostFilterPrev2
            | ParamKind::PostFilterPositiveBias
            | ParamKind::PostFilterNegativeBias => {
                &SEARCH_POST_DELTAS
            }
        }
    }

    fn apply(self, params: &mut AudioOutputParams, delta: i32) {
        match self {
            ParamKind::Deadzone => params.deadzone = (params.deadzone + delta).max(0),
            ParamKind::InputFilterCur => {
                params.input_filter_cur = (params.input_filter_cur + delta).clamp(120, 144)
            }
            ParamKind::InputFilterPrev => {
                params.input_filter_prev = (params.input_filter_prev + delta).clamp(-24, 24)
            }
            ParamKind::PrefilterGainNum => {
                params.prefilter_gain_num = (params.prefilter_gain_num + delta).clamp(120, 144)
            }
            ParamKind::CompressThresholdPositive => {
                params.compress_threshold_positive =
                    (params.compress_threshold_positive + delta).max(0)
            }
            ParamKind::CompressThresholdNegative => {
                params.compress_threshold_negative =
                    (params.compress_threshold_negative + delta).max(0)
            }
            ParamKind::CompressNumPositive => {
                params.compress_num_positive =
                    (params.compress_num_positive + delta).clamp(120, 128)
            }
            ParamKind::CompressNumNegative => {
                params.compress_num_negative =
                    (params.compress_num_negative + delta).clamp(120, 128)
            }
            ParamKind::PositiveBias => params.positive_bias += delta,
            ParamKind::NegativeBias => params.negative_bias += delta,
            ParamKind::PostFilterCur => params.post_filter_cur = (params.post_filter_cur + delta).clamp(120, 136),
            ParamKind::PostFilterPrev => params.post_filter_prev = (params.post_filter_prev + delta).clamp(-8, 8),
            ParamKind::PostFilterPrev2 => params.post_filter_prev2 = (params.post_filter_prev2 + delta).clamp(-8, 8),
            ParamKind::PostFilterPositiveBias => params.post_filter_positive_bias += delta,
            ParamKind::PostFilterNegativeBias => params.post_filter_negative_bias += delta,
            ParamKind::SignHysteresis => params.sign_hysteresis = (params.sign_hysteresis + delta).max(0),
            ParamKind::FinalFilterCur => params.final_filter_cur = (params.final_filter_cur + delta).clamp(120, 128),
            ParamKind::FinalFilterPrev => params.final_filter_prev = (params.final_filter_prev + delta).clamp(0, 16),
            ParamKind::FinalFilterPrev2 => params.final_filter_prev2 = (params.final_filter_prev2 + delta).clamp(-8, 4),
            ParamKind::FinalFilterPrev3 => params.final_filter_prev3 = (params.final_filter_prev3 + delta).clamp(-8, 4),
            ParamKind::FinalNonzeroBias => params.final_nonzero_bias += delta,
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
    let mut max_improvements: Option<usize> = None;
    let mut max_peak_overage: Option<i32> = None;
    let mut max_param_changes = 2usize;
    let mut peak_penalty_weight = 0.0f64;
    let mut require_first_nonzero_match = false;
    let mut raw_pair_input = false;
    let mut search_kinds = ParamKind::ALL.to_vec();
    let mut top_candidates: Option<usize> = None;
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
            "--max-improvements" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing max improvements value".to_string())?;
                max_improvements = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| "max improvements value must be a nonnegative integer".to_string())?,
                );
            }
            "--max-peak-overage" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing peak overage value".to_string())?;
                let parsed = value
                    .parse::<i32>()
                    .map_err(|_| "peak overage value must be an integer".to_string())?;
                if parsed < 0 {
                    return Err("peak overage value must be nonnegative".to_string());
                }
                max_peak_overage = Some(parsed);
            }
            "--max-param-changes" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing max param changes value".to_string())?;
                max_param_changes = value
                    .parse::<usize>()
                    .map_err(|_| "max param changes value must be between 1 and 3".to_string())?;
                if !(1..=3).contains(&max_param_changes) {
                    return Err("max param changes value must be between 1 and 3".to_string());
                }
            }
            "--peak-penalty-weight" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing peak penalty weight".to_string())?;
                peak_penalty_weight = value
                    .parse::<f64>()
                    .map_err(|_| "peak penalty weight must be a number".to_string())?;
                if peak_penalty_weight < 0.0 {
                    return Err("peak penalty weight must be nonnegative".to_string());
                }
            }
            "--require-first-nonzero-match" => require_first_nonzero_match = true,
            "--raw-pair-input" => raw_pair_input = true,
            "--only-params" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing only-params value".to_string())?;
                search_kinds = parse_param_kinds(&value)?;
            }
            "--top-candidates" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing top candidates value".to_string())?;
                top_candidates = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| "top candidates value must be a nonnegative integer".to_string())?,
                );
            }
            "--start-deadzone" => {
                start_params.deadzone = parse_i32_arg(args.next(), "missing deadzone value")?.max(0);
            }
            "--start-input-filter-cur" => {
                start_params.input_filter_cur =
                    parse_i32_arg(args.next(), "missing input-filter cur value")?.clamp(120, 144);
            }
            "--start-input-filter-prev" => {
                start_params.input_filter_prev =
                    parse_i32_arg(args.next(), "missing input-filter prev value")?.clamp(-24, 24);
            }
            "--start-prefilter-gain-num" => {
                start_params.prefilter_gain_num =
                    parse_i32_arg(args.next(), "missing prefilter gain numerator value")?.clamp(120, 144);
            }
            "--start-threshold" => {
                start_params.compress_threshold_positive =
                    parse_i32_arg(args.next(), "missing threshold value")?.max(0);
                start_params.compress_threshold_negative = start_params.compress_threshold_positive;
            }
            "--start-positive-threshold" => {
                start_params.compress_threshold_positive =
                    parse_i32_arg(args.next(), "missing positive threshold value")?.max(0);
            }
            "--start-negative-threshold" => {
                start_params.compress_threshold_negative =
                    parse_i32_arg(args.next(), "missing negative threshold value")?.max(0);
            }
            "--start-compress-num" => {
                start_params.compress_num_positive =
                    parse_i32_arg(args.next(), "missing compress numerator value")?.clamp(120, 128);
                start_params.compress_num_negative = start_params.compress_num_positive;
            }
            "--start-positive-compress-num" => {
                start_params.compress_num_positive =
                    parse_i32_arg(args.next(), "missing positive compress numerator value")?
                        .clamp(120, 128);
            }
            "--start-negative-compress-num" => {
                start_params.compress_num_negative =
                    parse_i32_arg(args.next(), "missing negative compress numerator value")?
                        .clamp(120, 128);
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
            "--start-post-filter-positive-bias" => {
                start_params.post_filter_positive_bias =
                    parse_i32_arg(args.next(), "missing post-filter positive bias value")?;
            }
            "--start-post-filter-negative-bias" => {
                start_params.post_filter_negative_bias =
                    parse_i32_arg(args.next(), "missing post-filter negative bias value")?;
            }
            "--start-sign-hysteresis" => {
                start_params.sign_hysteresis =
                    parse_i32_arg(args.next(), "missing sign hysteresis value")?.max(0);
            }
            "--start-final-filter-cur" => {
                start_params.final_filter_cur =
                    parse_i32_arg(args.next(), "missing final-filter cur value")?.clamp(120, 128);
            }
            "--start-final-filter-prev" => {
                start_params.final_filter_prev =
                    parse_i32_arg(args.next(), "missing final-filter prev value")?.clamp(0, 16);
            }
            "--start-final-filter-prev2" => {
                start_params.final_filter_prev2 =
                    parse_i32_arg(args.next(), "missing final-filter prev2 value")?.clamp(-8, 4);
            }
            "--start-final-filter-prev3" => {
                start_params.final_filter_prev3 =
                    parse_i32_arg(args.next(), "missing final-filter prev3 value")?.clamp(-8, 4);
            }
            "--start-final-nonzero-bias" => {
                start_params.final_nonzero_bias =
                    parse_i32_arg(args.next(), "missing final nonzero bias value")?;
            }
            _ => positional.push(arg),
        }
    }

    if positional.len() < 2 || positional.len() % 2 != 0 {
        return Err(
            "usage: tune_audio [--max-first-regression value] [--max-improvements count] [--max-peak-overage value] [--max-param-changes 1|2] [--peak-penalty-weight value] [--only-params key[,key...]] [--top-candidates count] [--raw-pair-input] <input.wav> <reference.wav> [<input.wav> <reference.wav> ...]"
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
            reference_peak: peak_abs(&reference_channel[..len]),
        });
    }

    let baseline_params = start_params;
    let baseline = score_candidate(&datasets, baseline_params, raw_pair_input, peak_penalty_weight);
    print_score("baseline", baseline_params, &datasets, &baseline);

    let best = search(
        &datasets,
        baseline_params,
        &baseline,
        max_first_regression,
        max_improvements,
        max_peak_overage,
        max_param_changes,
        peak_penalty_weight,
        require_first_nonzero_match,
        raw_pair_input,
        &search_kinds,
        top_candidates,
    );
    if let Some((best_params, best_score)) = best {
        print_score("best", best_params, &datasets, &best_score);
    } else {
        println!("no_candidate_matching_first_nonzero=true");
    }

    Ok(())
}

fn parse_i32_arg(value: Option<String>, missing: &str) -> Result<i32, String> {
    value
        .ok_or_else(|| missing.to_string())?
        .parse::<i32>()
        .map_err(|_| missing.replace("missing", "invalid"))
}

fn parse_param_kinds(value: &str) -> Result<Vec<ParamKind>, String> {
    let mut kinds = Vec::new();
    for token in value.split(|ch: char| ch == ',' || ch.is_whitespace()) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let kind = ParamKind::parse(token)
            .ok_or_else(|| format!("unknown param key for --only-params: {token}"))?;
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    if kinds.is_empty() {
        return Err("only-params must name at least one search key".to_string());
    }
    Ok(kinds)
}

fn search(
    datasets: &[Dataset],
    start: AudioOutputParams,
    baseline: &CandidateScore,
    max_first_regression: f64,
    max_improvements: Option<usize>,
    max_peak_overage: Option<i32>,
    max_param_changes: usize,
    peak_penalty_weight: f64,
    require_first_nonzero_match: bool,
    raw_pair_input: bool,
    search_kinds: &[ParamKind],
    top_candidates: Option<usize>,
) -> Option<(AudioOutputParams, CandidateScore)> {
    let mut best = if (require_first_nonzero_match && !matches_first_nonzero(datasets, baseline))
        || !matches_peak_overage(datasets, baseline, max_peak_overage)
    {
        None
    } else {
        Some((start, baseline.clone()))
    };
    let mut improvements = 0usize;

    loop {
        let mut improved = false;
        let mut round_best = best.clone();
        let mut round_top = Vec::new();
        let search_params = best
            .as_ref()
            .map(|(params, _)| *params)
            .unwrap_or(start);

        for (i, first) in search_kinds.iter().enumerate() {
            for &first_delta in first.deltas() {
                if first_delta == 0 {
                    continue;
                }
                let mut candidate = search_params;
                first.apply(&mut candidate, first_delta);
                consider_candidate(
                    datasets,
                    baseline,
                    max_first_regression,
                    max_peak_overage,
                    peak_penalty_weight,
                    require_first_nonzero_match,
                    raw_pair_input,
                    candidate,
                    &mut round_best,
                    &mut round_top,
                    top_candidates,
                );
            }
            if max_param_changes < 2 {
                continue;
            }
            for (j, second) in search_kinds.iter().enumerate().skip(i + 1) {
                for &first_delta in first.deltas() {
                    for &second_delta in second.deltas() {
                        if first_delta == 0 && second_delta == 0 {
                            continue;
                        }
                        let mut candidate = search_params;
                        first.apply(&mut candidate, first_delta);
                        second.apply(&mut candidate, second_delta);
                        consider_candidate(
                            datasets,
                            baseline,
                            max_first_regression,
                            max_peak_overage,
                            peak_penalty_weight,
                            require_first_nonzero_match,
                            raw_pair_input,
                            candidate,
                            &mut round_best,
                            &mut round_top,
                            top_candidates,
                        );
                    }
                }
                if max_param_changes < 3 {
                    continue;
                }
                for third in search_kinds.iter().skip(j + 1) {
                    for &first_delta in first.deltas() {
                        for &second_delta in second.deltas() {
                            for &third_delta in third.deltas() {
                                if first_delta == 0 && second_delta == 0 && third_delta == 0 {
                                    continue;
                                }
                                let mut candidate = search_params;
                                first.apply(&mut candidate, first_delta);
                                second.apply(&mut candidate, second_delta);
                                third.apply(&mut candidate, third_delta);
                                consider_candidate(
                                    datasets,
                                    baseline,
                                    max_first_regression,
                                    max_peak_overage,
                                    peak_penalty_weight,
                                    require_first_nonzero_match,
                                    raw_pair_input,
                                    candidate,
                                    &mut round_best,
                                    &mut round_top,
                                    top_candidates,
                                );
                            }
                        }
                    }
                }
            }
        }

        if top_candidates.unwrap_or(0) > 0 {
            for (idx, (params, score)) in round_top.iter().enumerate() {
                let label = format!("candidate[{idx}]");
                print_score(&label, *params, datasets, score);
            }
        }

        match (&best, &round_best) {
            (Some((_, best_score)), Some((round_best_params, round_best_score)))
                if round_best_score.objective_total + 1e-9 < best_score.objective_total =>
            {
                improved = true;
                best = Some((*round_best_params, round_best_score.clone()));
                improvements += 1;
                print_score("improved", *round_best_params, datasets, round_best_score);
            }
            (None, Some((round_best_params, round_best_score))) => {
                improved = true;
                best = Some((*round_best_params, round_best_score.clone()));
                improvements += 1;
                print_score("improved", *round_best_params, datasets, round_best_score);
            }
            _ => {}
        }

        if max_improvements.is_some_and(|limit| improvements >= limit) {
            return best;
        }

        if !improved {
            return best;
        }
    }
}

fn consider_candidate(
    datasets: &[Dataset],
    baseline: &CandidateScore,
    max_first_regression: f64,
    max_peak_overage: Option<i32>,
    peak_penalty_weight: f64,
    require_first_nonzero_match: bool,
    raw_pair_input: bool,
    params: AudioOutputParams,
    best: &mut Option<(AudioOutputParams, CandidateScore)>,
    top: &mut Vec<(AudioOutputParams, CandidateScore)>,
    top_limit: Option<usize>,
) {
    let score = score_candidate(datasets, params, raw_pair_input, peak_penalty_weight);
    if score.rmses[0] > baseline.rmses[0] + max_first_regression {
        return;
    }
    if require_first_nonzero_match && !matches_first_nonzero(datasets, &score) {
        return;
    }
    if !matches_peak_overage(datasets, &score, max_peak_overage) {
        return;
    }
    record_top_candidate(top, top_limit, params, &score);
    match best {
        Some((_, best_score)) if score.objective_total + 1e-9 < best_score.objective_total => {
            *best = Some((params, score));
        }
        None => {
            *best = Some((params, score));
        }
        _ => {}
    }
}

fn record_top_candidate(
    top: &mut Vec<(AudioOutputParams, CandidateScore)>,
    top_limit: Option<usize>,
    params: AudioOutputParams,
    score: &CandidateScore,
) {
    let Some(limit) = top_limit else {
        return;
    };
    if limit == 0 {
        return;
    }

    if let Some((_, existing_score)) = top.iter_mut().find(|(existing_params, _)| *existing_params == params) {
        if score.objective_total + 1e-9 < existing_score.objective_total {
            *existing_score = score.clone();
        } else {
            return;
        }
    } else {
        top.push((params, score.clone()));
    }

    top.sort_by(|(_, lhs), (_, rhs)| {
        lhs.objective_total
            .total_cmp(&rhs.objective_total)
            .then_with(|| lhs.total_rmse.total_cmp(&rhs.total_rmse))
            .then(lhs.total_peak_overage.cmp(&rhs.total_peak_overage))
    });
    if top.len() > limit {
        top.truncate(limit);
    }
}

fn matches_first_nonzero(datasets: &[Dataset], score: &CandidateScore) -> bool {
    score
        .first_nonzero_pairs
        .iter()
        .zip(datasets.iter())
        .all(|(pair, dataset)| *pair == dataset.reference_first_nonzero_pair)
}

fn matches_peak_overage(datasets: &[Dataset], score: &CandidateScore, max_peak_overage: Option<i32>) -> bool {
    let Some(max_peak_overage) = max_peak_overage else {
        return true;
    };
    score
        .peaks
        .iter()
        .zip(datasets.iter())
        .all(|(peak, dataset)| *peak <= dataset.reference_peak + max_peak_overage)
}

fn score_candidate(
    datasets: &[Dataset],
    params: AudioOutputParams,
    raw_pair_input: bool,
    peak_penalty_weight: f64,
) -> CandidateScore {
    let mut total_rmse = 0.0;
    let mut total_peak_overage = 0;
    let mut rmses = Vec::with_capacity(datasets.len());
    let mut first_nonzero_pairs = Vec::with_capacity(datasets.len());
    let mut peaks = Vec::with_capacity(datasets.len());
    for dataset in datasets {
        let (rmse, first_nonzero_pair, peak) = evaluate_dataset(dataset, params, raw_pair_input);
        total_rmse += rmse;
        total_peak_overage += (peak - dataset.reference_peak).max(0);
        rmses.push(rmse);
        first_nonzero_pairs.push(first_nonzero_pair);
        peaks.push(peak);
    }
    CandidateScore {
        total_rmse,
        objective_total: total_rmse + peak_penalty_weight * f64::from(total_peak_overage),
        total_peak_overage,
        rmses,
        first_nonzero_pairs,
        peaks,
    }
}

fn evaluate_dataset(dataset: &Dataset, params: AudioOutputParams, raw_pair_input: bool) -> (f64, usize, i32) {
    let mut input_history = 0i16;
    let mut delay_line = std::collections::VecDeque::with_capacity(AUDIO_OUTPUT_DELAY_PAIRS + 1);
    let mut filter_history = [0i32; AUDIO_OUTPUT_FILTER_TAPS.len() - 1];
    let mut post_history = 0i16;
    let mut post_history2 = 0i16;
    let mut last_nonzero_output = 0i16;
    let mut final_filter_history = [0i16; AUDIO_OUTPUT_FINAL_FILTER_TAPS.len() - 1];
    let mut error_sum = 0u128;
    let mut first_nonzero_pair = dataset.reference.len();
    let mut peak = 0i32;

    for (idx, (&scaled, &reference)) in dataset
        .prefilter
        .iter()
        .zip(dataset.reference.iter())
        .enumerate()
    {
        let input_sample = if raw_pair_input {
            delay_line.push_back(i32::from(scaled));
            let delayed = if delay_line.len() > AUDIO_OUTPUT_DELAY_PAIRS {
                delay_line.pop_front().unwrap_or(0)
            } else {
                0
            };
            delayed * AUDIO_OUTPUT_GAIN_NUM / AUDIO_OUTPUT_GAIN_DEN
        } else {
            i32::from(scaled)
        };
        let output = filter_audio_sample(
            input_sample,
            &mut input_history,
            &mut filter_history,
            &mut post_history,
            &mut post_history2,
            &mut last_nonzero_output,
            &mut final_filter_history,
            params,
        );
        if output != 0 && first_nonzero_pair == dataset.reference.len() {
            first_nonzero_pair = idx;
        }
        peak = peak.max(i32::from(output).abs());
        let error = i64::from(output) - i64::from(reference);
        error_sum += (error * error) as u128;
    }

    (
        (error_sum as f64 / dataset.reference.len() as f64).sqrt(),
        first_nonzero_pair,
        peak,
    )
}

fn filter_audio_sample(
    input_sample: i32,
    input_history: &mut i16,
    filter_history: &mut [i32; AUDIO_OUTPUT_FILTER_TAPS.len() - 1],
    post_history: &mut i16,
    post_history2: &mut i16,
    last_nonzero_output: &mut i16,
    final_filter_history: &mut [i16; AUDIO_OUTPUT_FINAL_FILTER_TAPS.len() - 1],
    params: AudioOutputParams,
) -> i16 {
    let filtered_input = round_divide(
        input_sample * params.input_filter_cur + i32::from(*input_history) * params.input_filter_prev,
        AUDIO_OUTPUT_INPUT_FILTER_DEN,
    )
    .clamp(i16::MIN as i32, i16::MAX as i32);
    *input_history = input_sample.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    let scaled = round_divide(filtered_input * params.prefilter_gain_num, AUDIO_OUTPUT_PREFILTER_GAIN_DEN);
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
    let compressed = if gained > params.compress_threshold_positive {
        let above = gained - params.compress_threshold_positive;
        params.compress_threshold_positive
            + round_divide(above * params.compress_num_positive, AUDIO_OUTPUT_COMPRESS_DEN)
    } else if gained < -params.compress_threshold_negative {
        let above = (-gained) - params.compress_threshold_negative;
        -(params.compress_threshold_negative
            + round_divide(above * params.compress_num_negative, AUDIO_OUTPUT_COMPRESS_DEN))
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
    let mut output = if post_filtered == 0 {
        0
    } else if post_filtered > 0 {
        (post_filtered + params.post_filter_positive_bias)
            .clamp(i16::MIN as i32, i16::MAX as i32) as i16
    } else {
        (post_filtered + params.post_filter_negative_bias)
            .clamp(i16::MIN as i32, i16::MAX as i32) as i16
    };
    if *last_nonzero_output != 0
        && output != 0
        && (*last_nonzero_output > 0) != (output > 0)
        && i32::from(output).abs() <= params.sign_hysteresis
    {
        output = 0;
    }
    if output != 0 {
        *last_nonzero_output = output;
    }
    let mut final_accum = params.final_filter_cur * i32::from(output);
    final_accum += params.final_filter_prev * i32::from(final_filter_history[0]);
    final_accum += params.final_filter_prev2 * i32::from(final_filter_history[1]);
    final_accum += params.final_filter_prev3 * i32::from(final_filter_history[2]);
    let corrected_output =
        round_divide(final_accum, AUDIO_OUTPUT_FINAL_FILTER_DEN).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    let history_len = final_filter_history.len();
    final_filter_history.copy_within(0..history_len - 1, 1);
    final_filter_history[0] = output;
    if corrected_output == 0 {
        0
    } else {
        (i32::from(corrected_output) + params.final_nonzero_bias)
            .clamp(i16::MIN as i32, i16::MAX as i32) as i16
    }
}

fn print_score(label: &str, params: AudioOutputParams, datasets: &[Dataset], score: &CandidateScore) {
    println!(
        "{label} dead={} icur={} iprev={} pregain={} pthr={} nthr={} pcnum={} ncnum={} pos_bias={} neg_bias={} cur={} prev={} prev2={} post_pos_bias={} post_neg_bias={} sign_hyst={} fcur={} fprev={} fprev2={} fprev3={} fnonzero={} total_rmse={:.6} objective_total={:.6} total_peak_overage={}",
        params.deadzone,
        params.input_filter_cur,
        params.input_filter_prev,
        params.prefilter_gain_num,
        params.compress_threshold_positive,
        params.compress_threshold_negative,
        params.compress_num_positive,
        params.compress_num_negative,
        params.positive_bias,
        params.negative_bias,
        params.post_filter_cur,
        params.post_filter_prev,
        params.post_filter_prev2,
        params.post_filter_positive_bias,
        params.post_filter_negative_bias,
        params.sign_hysteresis,
        params.final_filter_cur,
        params.final_filter_prev,
        params.final_filter_prev2,
        params.final_filter_prev3,
        params.final_nonzero_bias,
        score.total_rmse,
        score.objective_total,
        score.total_peak_overage,
    );
    for (((dataset, rmse), first_nonzero_pair), peak) in datasets
        .iter()
        .zip(score.rmses.iter())
        .zip(score.first_nonzero_pairs.iter())
        .zip(score.peaks.iter())
    {
        println!(
            "  rmse {:.6} first_nonzero={} ref={} peak={} ref_peak={} {}",
            rmse,
            first_nonzero_pair,
            dataset.reference_first_nonzero_pair,
            peak,
            dataset.reference_peak,
            dataset.label
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

fn peak_abs(samples: &[i16]) -> i32 {
    samples.iter().map(|sample| i32::from(*sample).abs()).max().unwrap_or(0)
}

fn round_divide(value: i32, denominator: i32) -> i32 {
    if value >= 0 {
        (value + denominator / 2) / denominator
    } else {
        -((-value + denominator / 2) / denominator)
    }
}
