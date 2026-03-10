pub struct Enclave {}

impl Enclave {
    pub fn new() -> Self {
        Self {}
    }

    pub fn start(&self) {
        // TODO: start the enclave (e.g. run data-plane in a subprocess or connect to existing enclave)
    }

    pub fn terminate(&self) {
        // TODO: stop the enclave
    }
}

impl Drop for Enclave {
    fn drop(&mut self) {
        self.terminate();
    }
}
