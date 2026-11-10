//! Independent QA: compare vertical image motion in fixed native screenshot ROIs.
//! This reads saved screenshots only; it neither captures nor controls the UI.
use serde_json::json;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        return Err("usage: screenshot_motion before.jpg after.jpg x,y,width,height [...]".into());
    }
    let before = image::open(&args[1])?.to_luma8();
    let after = image::open(&args[2])?.to_luma8();
    let range = std::env::var("TRAVEL_GALLERY_MOTION_RANGE")
        .unwrap_or_else(|_| "100".into())
        .parse::<u32>()?;
    if !(1..=1000).contains(&range) {
        return Err("search range must be between 1 and 1000 device pixels".into());
    }
    if before.dimensions() != after.dimensions() {
        return Err("screenshots must have the same pixel dimensions".into());
    }
    let mut observations = vec![];
    for region in &args[3..] {
        let rect = region
            .split(',')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()?;
        if rect.len() != 4 {
            return Err("ROI needs x,y,width,height".into());
        }
        let (x, y, w, h) = (rect[0], rect[1], rect[2], rect[3]);
        if w == 0
            || h == 0
            || x + w > before.width()
            || y < range
            || y + h + range > before.height()
        {
            return Err("ROI or its search margin is out of bounds".into());
        }
        let difference = |shift: i32| {
            let mut total = 0u64;
            let mut count = 0u64;
            for py in (y..y + h).step_by(4) {
                for px in (x..x + w).step_by(4) {
                    total += u8::abs_diff(
                        after[(px, py)][0],
                        before[(px, (py as i32 - shift) as u32)][0],
                    ) as u64;
                    count += 1;
                }
            }
            total as f64 / count as f64
        };
        let (shift, score) = (-(range as i32)..=range as i32)
            .map(|delta| (delta, difference(delta)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        observations.push(json!({"region":rect,"verticalShiftDevicePixels":shift,"meanAbsoluteDifferenceAtShift":score,"meanAbsoluteDifferenceWithoutShift":difference(0)}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"dimensions":[before.width(),before.height()],"sampleStep":4,"searchRangeDevicePixels":[-(range as i32),range],"observations":observations})
        )?
    );
    Ok(())
}
