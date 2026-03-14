use enigo::{Direction, Enigo, Key, Keyboard, Mouse, Settings};

#[allow(dead_code)]
pub struct InputFallback {
    enigo: Enigo,
}

#[allow(dead_code)]
impl InputFallback {
    pub fn new() -> anyhow::Result<Self> {
        let enigo = Enigo::new(&Settings::default())?;
        Ok(Self { enigo })
    }

    pub fn text(&mut self, value: &str) -> anyhow::Result<()> {
        self.enigo.text(value)?;
        Ok(())
    }

    pub fn press_enter(&mut self) -> anyhow::Result<()> {
        self.enigo.key(Key::Return, Direction::Click)?;
        Ok(())
    }

    pub fn left_click(&mut self) -> anyhow::Result<()> {
        self.enigo.button(enigo::Button::Left, Direction::Click)?;
        Ok(())
    }
}
