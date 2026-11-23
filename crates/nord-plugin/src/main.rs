use nih_plug::prelude::*;
use nord_plugin::NdMatrix1Plugin;

fn main() {
    nih_export_standalone::<NdMatrix1Plugin>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standalone_target_symbols() {
        assert_eq!(NdMatrix1Plugin::NAME, "ND Matrix 1");
    }
}
