use iced::widget::{button}

pub fn main() -> iced::Result {
    iced::run(Brush::update, Brush::view)
}

struct Brush {

}
