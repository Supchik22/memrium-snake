//! Memrium Snake: the old game on a little green LCD, in a handheld's body. Arrow keys (or W A S D, or the pad on the
//! screen) steer, Space pauses, Enter starts a new game.

use chebrium::app::{self, App, Config, Decorations, Host};
use chebrium::chrome;
use chebrium::draw::Align;
use chebrium::input::{Key, KeyMods};
use chebrium::retro::{digit_cell, key_body};
use chebrium::ui::Ui;
use std::collections::VecDeque;
use std::time::Instant;

const COLS: i32 = 16;
const ROWS: i32 = 12;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    fn opposite(self) -> Dir {
        match self {
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
        }
    }

    fn delta(self) -> (i32, i32) {
        match self {
            Dir::Up => (0, -1),
            Dir::Down => (0, 1),
            Dir::Left => (-1, 0),
            Dir::Right => (1, 0),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Step {
    Moved,
    Ate,
    Died,
    /// the snake fills the whole board
    Won,
}

/// The game itself, independent of how it is drawn.
struct Game {
    snake: VecDeque<(i32, i32)>,
    dir: Dir,
    /// turns waiting for their step (a quick double turn must not be lost, nor turn the snake back on itself)
    queue: Vec<Dir>,
    food: (i32, i32),
    score: u32,
    rng: u64,
}

impl Game {
    fn new(seed: u64) -> Game {
        let mid = (COLS / 2, ROWS / 2);
        let snake = VecDeque::from([(mid.0 + 1, mid.1), mid, (mid.0 - 1, mid.1)]);
        let mut g = Game { snake, dir: Dir::Right, queue: Vec::new(), food: (0, 0), score: 0, rng: seed | 1 };
        g.place_food();
        g
    }

    fn next_rand(&mut self) -> u64 {
        // xorshift: plenty for choosing where the food goes
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    /// Put the food on a free cell. False when there is none.
    fn place_food(&mut self) -> bool {
        let free: Vec<(i32, i32)> = (0..ROWS).flat_map(|y| (0..COLS).map(move |x| (x, y))).filter(|c| !self.snake.contains(c)).collect();
        if free.is_empty() {
            return false;
        }
        let i = (self.next_rand() % free.len() as u64) as usize;
        self.food = free[i];
        true
    }

    /// Steer: a turn back on itself, or the same way again, is ignored; at most two turns wait.
    fn turn(&mut self, d: Dir) {
        let last = self.queue.last().copied().unwrap_or(self.dir);
        if d != last && d != last.opposite() && self.queue.len() < 2 {
            self.queue.push(d);
        }
    }

    fn step(&mut self) -> Step {
        if !self.queue.is_empty() {
            self.dir = self.queue.remove(0);
        }
        let (dx, dy) = self.dir.delta();
        let head = self.snake[0];
        let next = (head.0 + dx, head.1 + dy);
        if next.0 < 0 || next.1 < 0 || next.0 >= COLS || next.1 >= ROWS {
            return Step::Died;
        }
        let ate = next == self.food;
        // the tail moves out of the way, unless the snake grows
        let body = self.snake.len() - usize::from(!ate);
        if self.snake.iter().take(body).any(|c| *c == next) {
            return Step::Died;
        }
        self.snake.push_front(next);
        if ate {
            self.score += 1;
            if !self.place_food() {
                return Step::Won;
            }
            Step::Ate
        } else {
            self.snake.pop_back();
            Step::Moved
        }
    }

    /// Seconds between steps: quicker as the snake grows.
    fn interval(&self) -> f32 {
        (0.17 - 0.004 * self.score as f32).max(0.075)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Ready,
    Playing,
    Paused,
    Over { won: bool },
}

fn hi_path() -> std::path::PathBuf {
    memrium_de::theme::config_dir().join("snake-hi")
}

const BODY: [f32; 3] = [0.925, 0.915, 0.855];
const KEY_INK: [f32; 3] = [0.40, 0.40, 0.36];
const PIXEL: [f32; 3] = [0.09, 0.14, 0.06];

struct SnakeApp {
    game: Game,
    phase: Phase,
    acc: f32,
    last: Instant,
    clock: f32,
    hi: u32,
    flash: Option<usize>,
}

impl SnakeApp {
    fn new() -> SnakeApp {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7);
        let hi = std::fs::read_to_string(hi_path()).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0);
        SnakeApp { game: Game::new(seed), phase: Phase::Ready, acc: 0.0, last: Instant::now(), clock: 0.0, hi, flash: None }
    }

    fn start(&mut self) {
        match self.phase {
            Phase::Ready | Phase::Over { .. } => {
                let seed = self.game.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                self.game = Game::new(seed);
                self.acc = 0.0;
                self.phase = Phase::Playing;
            }
            Phase::Playing => self.phase = Phase::Paused,
            Phase::Paused => self.phase = Phase::Playing,
        }
    }

    fn restart(&mut self) {
        self.phase = Phase::Ready;
        self.start();
    }

    fn steer(&mut self, d: Dir) {
        match self.phase {
            Phase::Playing => self.game.turn(d),
            // a turn starts a game that has not begun
            Phase::Ready => {
                self.start();
                self.game.turn(d);
            }
            _ => {}
        }
    }

    fn finish(&mut self, won: bool) {
        self.phase = Phase::Over { won };
        if self.game.score > self.hi {
            self.hi = self.game.score;
            let _ = std::fs::create_dir_all(hi_path().parent().unwrap_or(std::path::Path::new(".")));
            let _ = std::fs::write(hi_path(), self.hi.to_string());
        }
    }
}

/// A number on the LCD, four seven-segment digits with the unlit ones faintly there.
fn lcd_number(ui: &mut Ui<'_>, right: f32, y: f32, h: f32, n: u32) {
    let (w, pitch) = (h * 0.52, h * 0.52 + h * 0.20);
    let text = format!("{:>4}", n.min(9999));
    for (i, ch) in text.chars().rev().enumerate() {
        let x = right - w - i as f32 * pitch;
        digit_cell(ui, x, y, w, h, ' ', PIXEL);
        digit_cell(ui, x, y, w, h, ch, PIXEL);
    }
}

impl App for SnakeApp {
    fn tick(&mut self, host: &mut Host) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;
        self.clock += dt;
        match self.phase {
            Phase::Playing => {
                self.acc += dt;
                while self.acc >= self.game.interval() {
                    self.acc -= self.game.interval();
                    match self.game.step() {
                        Step::Died => {
                            self.finish(false);
                            break;
                        }
                        Step::Won => {
                            self.finish(true);
                            break;
                        }
                        _ => {}
                    }
                }
                host.redraw = true;
            }
            // the waiting message blinks
            Phase::Ready => host.redraw = true,
            _ => {}
        }
    }

    fn key(&mut self, key: Key, _mods: KeyMods, host: &mut Host) {
        match key {
            Key::Up | Key::Char('w' | 'W') => self.steer(Dir::Up),
            Key::Down | Key::Char('s' | 'S') => self.steer(Dir::Down),
            Key::Left | Key::Char('a' | 'A') => self.steer(Dir::Left),
            Key::Right | Key::Char('d' | 'D') => self.steer(Dir::Right),
            Key::Char(' ') | Key::Char('p' | 'P') => {
                if self.phase != Phase::Ready {
                    self.start();
                } else {
                    self.start();
                }
            }
            Key::Enter => self.restart(),
            Key::Esc => {
                host.quit = true;
                return;
            }
            _ => return,
        }
        host.redraw = true;
    }

    fn draw(&mut self, ui: &mut Ui<'_>, size: [f32; 2], host: &mut Host) {
        let [w, h] = size;
        ui.fill([0.0, 0.0, w, h], 0.0, BODY, 1.0);
        let resizing = chrome::resize_borders(ui, host, size);
        let ink = ui.ink;
        ui.ink = [0.25, 0.25, 0.22];
        let head = chrome::titlebar(ui, host, w, &chebrium::i18n::tr("Snake"));
        ui.ink = ink;
        let press_ok = !resizing && ui.mouse[1] > head;
        let pad = (w * 0.06).clamp(14.0, 26.0);

        // ---- sizes: the pad has what it needs, the screen takes the rest
        let d = 52.0f32;
        let ctrl_h = 3.0 * d + 12.0;
        let top_gap = 14.0;
        let strip = 46.0;
        let avail = h - head - top_gap - 22.0 - ctrl_h - pad;
        let cell = ((w - 2.0 * pad - 24.0) / COLS as f32).min((avail - strip - 16.0) / ROWS as f32).max(8.0);
        let (grid_w, grid_h) = (cell * COLS as f32, cell * ROWS as f32);
        let lcd = [pad, head + top_gap, grid_w + 24.0, strip + grid_h + 16.0];
        let lcd = [(w - lcd[2]) / 2.0, lcd[1], lcd[2], lcd[3]];

        // ---- the LCD: green, in a darker frame, a strip of scores above the field
        ui.fill([lcd[0] - 6.0, lcd[1] - 6.0, lcd[2] + 12.0, lcd[3] + 12.0], 16.0, [0.45, 0.47, 0.42], 0.60);
        ui.grad(lcd, 10.0, [0.64, 0.73, 0.47], [0.72, 0.80, 0.55], 1.0);
        ui.fill([lcd[0], lcd[1], lcd[2], 3.0], 2.0, [0.0; 3], 0.10);
        ui.text_raw(lcd[0] + 14.0, lcd[1] + 16.0, "SCORE", 9.5, true, PIXEL, 0.8, Align::Left);
        lcd_number(ui, lcd[0] + 14.0 + 4.0 * 20.0, lcd[1] + 22.0, 20.0, self.game.score);
        ui.text_raw(lcd[0] + lcd[2] - 14.0 - 4.0 * 20.0 - 6.0, lcd[1] + 16.0, "HI", 9.5, true, PIXEL, 0.8, Align::Right);
        lcd_number(ui, lcd[0] + lcd[2] - 14.0, lcd[1] + 22.0, 20.0, self.hi.max(if matches!(self.phase, Phase::Over { .. }) { 0 } else { 0 }));
        // the field, a hairline round it, the faint ghost of every cell
        let field = [lcd[0] + 12.0, lcd[1] + strip, grid_w, grid_h];
        ui.fill([field[0] - 3.0, field[1] - 3.0, field[2] + 6.0, field[3] + 6.0], 3.0, PIXEL, 0.55);
        ui.fill([field[0] - 1.5, field[1] - 1.5, field[2] + 3.0, field[3] + 3.0], 2.0, [0.69, 0.77, 0.52], 1.0);
        for y in 0..ROWS {
            for x in 0..COLS {
                let r = [field[0] + x as f32 * cell + 1.0, field[1] + y as f32 * cell + 1.0, cell - 2.0, cell - 2.0];
                ui.fill(r, cell * 0.18, PIXEL, 0.055);
            }
        }
        let on = |ui: &mut Ui<'_>, x: i32, y: i32, inset: f32, a: f32| {
            let r = [field[0] + x as f32 * cell + 1.0 + inset, field[1] + y as f32 * cell + 1.0 + inset, cell - 2.0 - 2.0 * inset, cell - 2.0 - 2.0 * inset];
            ui.fill(r, cell * 0.18, PIXEL, a);
        };
        let blink = (self.clock * 2.4).fract() < 0.6;
        match self.phase {
            Phase::Ready => {
                // a short snake waiting to be started
                for i in 0..5 {
                    on(ui, 5 + i, ROWS / 2, 0.0, 0.92);
                }
                on(ui, 12, ROWS / 2, cell * 0.12, 0.92);
                if blink {
                    ui.text_raw(field[0] + field[2] / 2.0, field[1] + cell * 3.2, "PRESS START", cell * 0.9, true, PIXEL, 0.9, Align::Center);
                }
            }
            _ => {
                for (i, &(x, y)) in self.game.snake.iter().enumerate() {
                    on(ui, x, y, if i == 0 { 0.0 } else { cell * 0.04 }, 0.92);
                }
                // the food blinks, a little smaller than a cell of the snake
                let (fx, fy) = self.game.food;
                if blink || self.phase != Phase::Playing {
                    on(ui, fx, fy, cell * 0.16, 0.92);
                }
                if let Phase::Over { won } = self.phase {
                    let r = [field[0] + cell * 2.0, field[1] + field[3] / 2.0 - cell * 1.4, field[2] - cell * 4.0, cell * 2.8];
                    ui.fill(r, 6.0, [0.69, 0.77, 0.52], 0.96);
                    ui.fill([r[0], r[1], r[2], 2.0], 1.0, PIXEL, 0.9);
                    ui.fill([r[0], r[1] + r[3] - 2.0, r[2], 2.0], 1.0, PIXEL, 0.9);
                    ui.text_raw(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0 + cell * 0.1, if won { "YOU WIN" } else { "GAME OVER" }, cell * 1.0, true, PIXEL, 0.95, Align::Center);
                } else if self.phase == Phase::Paused {
                    ui.text_raw(field[0] + field[2] / 2.0, field[1] + field[3] / 2.0, "PAUSE", cell * 1.1, true, PIXEL, 0.9, Align::Center);
                }
            }
        }

        // ---- the pad and the two buttons
        let top = lcd[1] + lcd[3] + 28.0;
        let pale = [0.955, 0.945, 0.90];
        let mut pressed: Option<(usize, u8)> = None;
        let mut key = |ui: &mut Ui<'_>, c: [f32; 2], dia: f32, slot: usize, label: &str, face: [f32; 3], code: u8| {
            let r = [c[0] - dia / 2.0, c[1] - dia / 2.0, dia, dia];
            let hov = ui.hit(r);
            let wash = ui.ease(10_000 + slot as u64, if hov { 1.0 } else { 0.0 }, 20.0);
            let flash = ui.ease(20_000 + slot as u64, if self.flash == Some(slot) { 1.0 } else { 0.0 }, 28.0);
            key_body(ui, r, face, wash, flash);
            // (a word on a key is smaller than a sign)
            let size = if label.chars().count() > 2 { dia * 0.27 } else { dia * 0.40 };
            ui.text_raw(c[0], c[1] + size * 0.34, label, size, false, KEY_INK, 0.95, Align::Center);
            if ui.click && hov && press_ok {
                pressed = Some((slot, code));
            }
        };
        let pc = [pad + d * 1.5 + 6.0, top + ctrl_h / 2.0];
        let o = d + 4.0;
        key(ui, [pc[0], pc[1] - o], d, 0, "▲", pale, 0);
        key(ui, [pc[0], pc[1] + o], d, 1, "▼", pale, 1);
        key(ui, [pc[0] - o, pc[1]], d, 2, "◀", pale, 2);
        key(ui, [pc[0] + o, pc[1]], d, 3, "▶", pale, 3);
        // the two buttons, one lower and to the left, one higher and to the right, each with its name under it
        let (db, my) = (d * 1.05, top + ctrl_h / 2.0);
        let right = w - pad - db / 2.0;
        let (start_c, new_c) = ([right - db * 1.1, my + db * 0.55], [right, my - db * 0.55]);
        key(ui, start_c, db, 4, "II", [0.62, 0.84, 0.42], 4);
        key(ui, new_c, db, 5, "NEW", pale, 5);
        ui.text_raw(start_c[0], start_c[1] + db / 2.0 + 14.0, "START / PAUSE", 9.0, true, KEY_INK, 0.7, Align::Center);
        ui.text_raw(new_c[0], new_c[1] + db / 2.0 + 14.0, "NEW GAME", 9.0, true, KEY_INK, 0.7, Align::Center);
        if let Some((slot, code)) = pressed {
            match code {
                0 => self.steer(Dir::Up),
                1 => self.steer(Dir::Down),
                2 => self.steer(Dir::Left),
                3 => self.steer(Dir::Right),
                4 => self.start(),
                _ => self.restart(),
            }
            self.flash = Some(slot);
            host.redraw = true;
        }
        if let Some(slot) = self.flash {
            if ui.ease(20_000 + slot as u64, 1.0, 28.0) > 0.6 {
                self.flash = None;
                host.redraw = true;
            }
        }
    }
}

fn main() {
    app::run(Config::new("Memrium Snake", "memrium-snake").size(340, 600).min_size(300, 540).decorations(Decorations::Client), SnakeApp::new());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> Game {
        let mut g = Game::new(12345);
        // food out of the way for the tests that only move
        g.food = (0, 0);
        g
    }

    #[test]
    fn it_moves_one_cell_a_step_keeping_its_length() {
        let mut g = game();
        let head = g.snake[0];
        assert_eq!(g.step(), Step::Moved);
        assert_eq!(g.snake[0], (head.0 + 1, head.1));
        assert_eq!(g.snake.len(), 3);
    }

    #[test]
    fn eating_grows_it_and_scores_and_moves_the_food() {
        let mut g = game();
        let head = g.snake[0];
        g.food = (head.0 + 1, head.1);
        assert_eq!(g.step(), Step::Ate);
        assert_eq!((g.snake.len(), g.score), (4, 1));
        assert!(!g.snake.contains(&g.food));
    }

    #[test]
    fn hitting_a_wall_ends_the_game() {
        let mut g = game();
        for _ in 0..COLS {
            if g.step() == Step::Died {
                return;
            }
        }
        panic!("the snake went through the wall");
    }

    #[test]
    fn it_cannot_turn_back_on_itself() {
        let mut g = game();
        g.turn(Dir::Left);
        assert_eq!(g.step(), Step::Moved);
        assert_eq!(g.dir, Dir::Right);
    }

    #[test]
    fn two_quick_turns_are_both_kept_in_order() {
        let mut g = game();
        g.turn(Dir::Down);
        g.turn(Dir::Left);
        g.step();
        assert_eq!(g.dir, Dir::Down);
        g.step();
        assert_eq!(g.dir, Dir::Left);
    }

    #[test]
    fn running_into_itself_ends_the_game() {
        let mut g = game();
        // a snake coiled so that the next step into its own body is a hit
        g.snake = VecDeque::from([(5, 5), (5, 6), (4, 6), (4, 5), (4, 4), (5, 4)]);
        g.dir = Dir::Up;
        g.queue.clear();
        g.food = (0, 0);
        g.turn(Dir::Left);
        assert_eq!(g.step(), Step::Died);
    }

    #[test]
    fn following_its_own_tail_is_fine() {
        let mut g = game();
        g.snake = VecDeque::from([(5, 5), (5, 6), (4, 6), (4, 5)]);
        g.dir = Dir::Left;
        g.food = (0, 0);
        // the head moves into the cell the tail leaves
        assert_eq!(g.step(), Step::Moved);
    }

    #[test]
    fn the_food_is_never_on_the_snake() {
        for seed in 1..200u64 {
            let g = Game::new(seed * 7919);
            assert!(!g.snake.contains(&g.food));
        }
    }

    #[test]
    fn it_gets_quicker() {
        let mut g = game();
        let slow = g.interval();
        g.score = 20;
        assert!(g.interval() < slow);
        g.score = 1000;
        assert!(g.interval() >= 0.075);
    }
}
