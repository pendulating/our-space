//! One-off: report the peak concurrent rideshare trips across the baked day, to
//! size the on-screen vehicle cap. Run:
//!   cargo run -p sim-core --example taxi_peak -- crates/app-interactive/assets/processed/taxi_day_20260421.ostaxiday

use sim_core::assets::TaxiDayLayer;

fn main() {
    let path = std::env::args().nth(1).expect("usage: taxi_peak <file.ostaxiday>");
    let bytes = std::fs::read(&path).expect("read file");
    let layer = TaxiDayLayer::from_bytes(&bytes).expect("decode TaxiDayLayer");

    const N: usize = 1440;
    let mut diff = vec![0i64; N + 1];
    for t in &layer.trips {
        let s = (t.pu_min.floor() as i64).clamp(0, N as i64) as usize;
        let e = ((t.pu_min + t.dur_min).ceil() as i64).clamp(0, N as i64) as usize;
        diff[s] += 1;
        diff[e.max(s)] -= 1;
    }
    let mut conc = vec![0i64; N];
    let mut cur = 0i64;
    for m in 0..N {
        cur += diff[m];
        conc[m] = cur;
    }
    let (peak, peak_m) = conc.iter().enumerate().fold((0i64, 0), |(mx, mi), (i, &c)| {
        if c > mx { (c, i) } else { (mx, mi) }
    });
    let mean: f64 = conc.iter().sum::<i64>() as f64 / N as f64;
    let mut sorted = conc.clone();
    sorted.sort_unstable();
    let median = sorted[N / 2];
    let p95 = sorted[(N as f64 * 0.95) as usize];
    let over = |k: i64| conc.iter().filter(|&&c| c > k).count();

    println!("baked trips:            {}", layer.trips.len());
    println!("PEAK concurrent:        {peak}  at {:02}:{:02}", peak_m / 60, peak_m % 60);
    println!("mean concurrent (24h):  {mean:.0}");
    println!("median / p95:           {median} / {p95}");
    println!("minutes over 1500:      {} ({} h)", over(1500), over(1500) / 60);
    println!("minutes over 3000:      {}", over(3000));
    println!("minutes over 5000:      {}", over(5000));
    // Effective taxi pace = routed length / real trip duration (what the replay
    // animates), to compare against the fixed agent speeds (ped 1.34, robot 1.8,
    // tesla 8.0 m/s). The clock-rate time-lapse multiplies all of these equally.
    let mut speeds: Vec<f64> = layer
        .trips
        .iter()
        .filter_map(|t| {
            let r = layer.routes.get(t.route_idx as usize)?;
            let dur_s = t.dur_min as f64 * 60.0;
            (dur_s > 0.0 && r.length_m > 0.0).then(|| r.length_m as f64 / dur_s)
        })
        .collect();
    speeds.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| speeds[((speeds.len() as f64 * p) as usize).min(speeds.len() - 1)];
    let mean_sp = speeds.iter().sum::<f64>() / speeds.len() as f64;
    println!(
        "taxi effective speed m/s (per-trip):  p10={:.1}  median={:.1}  mean={:.1}  p90={:.1}",
        pct(0.1),
        pct(0.5),
        mean_sp,
        pct(0.9)
    );

    // What you actually SEE: each trip occupies an on-screen slot for its whole
    // duration, so slow trips are over-represented at any given instant. Weight the
    // speed distribution by dur_min to get the time-averaged on-screen pace — the
    // number the synthetic Tesla/robot speeds should match to "flow with traffic".
    let mut wspeeds: Vec<(f64, f64)> = layer
        .trips
        .iter()
        .filter_map(|t| {
            let r = layer.routes.get(t.route_idx as usize)?;
            let dur_s = t.dur_min as f64 * 60.0;
            (dur_s > 0.0 && r.length_m > 0.0).then(|| (r.length_m as f64 / dur_s, dur_s))
        })
        .collect();
    wspeeds.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let wtot: f64 = wspeeds.iter().map(|x| x.1).sum();
    let wpct = |p: f64| {
        let target = wtot * p;
        let mut acc = 0.0;
        for &(s, w) in &wspeeds {
            acc += w;
            if acc >= target {
                return s;
            }
        }
        wspeeds.last().map(|x| x.0).unwrap_or(0.0)
    };
    let wmean = wspeeds.iter().map(|(s, w)| s * w).sum::<f64>() / wtot;
    println!(
        "taxi on-screen pace m/s (dur-weighted):  p10={:.1}  median={:.1}  mean={:.1}  p90={:.1}",
        wpct(0.1),
        wpct(0.5),
        wmean,
        wpct(0.9)
    );

    // hourly peak profile
    print!("hourly peak: ");
    for h in 0..24 {
        let hp = conc[h * 60..(h + 1) * 60].iter().max().copied().unwrap_or(0);
        print!("{h:02}={hp} ");
    }
    println!();
}
