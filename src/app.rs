use rarog_engine::{Engine, EngineError};

pub struct ZoryaApp {
    engine: Engine,
}

impl ZoryaApp {
    pub fn bootstrap() -> Result<Self, EngineError> {
        Ok(Self {
            engine: Engine::builder().build()?,
        })
    }

    pub const fn engine(&self) -> &Engine {
        &self.engine
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_initializes_rarog_engine() {
        assert!(ZoryaApp::bootstrap().is_ok());
    }
}
