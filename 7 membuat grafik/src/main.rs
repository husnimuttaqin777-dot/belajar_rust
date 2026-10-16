use rand::Rng;
use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};

slint::include_modules!();

const W: f32 = 1000.0; // sama dengan viewbox-width
const H: f32 = 400.0;  // sama dengan viewbox-height

fn random_data(n: usize) -> Vec<f32> {
    let mut rng = rand::thread_rng();
    (0..n).map(|_| rng.gen_range(0.0..100.0)).collect()
}

fn make_path(values: &[f32]) -> String {
    let step = W / (values.len() - 1) as f32;
    let mut s = String::new();
    for (i, v) in values.iter().enumerate() {
        let x = i as f32 * step;
        let y = H - (v / 100.0) * H;
        let cmd = if i == 0 { "M" } else { "L" };
        s.push_str(&format!("{cmd} {x:.1} {y:.1} "));
    }
    s
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    ui.set_commands(make_path(&random_data(50)).into());

    // tombol "Generate data baru"
    let weak = ui.as_weak();
    ui.on_regenerate(move || {
        let ui = weak.unwrap();
        ui.set_commands(make_path(&random_data(50)).into());
    });

    // grafik bergerak real-time
    let data: Rc<RefCell<VecDeque<f32>>> =
        Rc::new(RefCell::new(random_data(50).into_iter().collect()));

    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let d = data.clone();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
        let mut d = d.borrow_mut();
        d.pop_front();
        d.push_back(rand::thread_rng().gen_range(0.0..100.0));
        let v: Vec<f32> = d.iter().copied().collect();
        weak.unwrap().set_commands(make_path(&v).into());
    });

    ui.run()
}