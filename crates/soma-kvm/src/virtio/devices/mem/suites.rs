//! The independent test suites of the memory device (bounds, atomicity, wire, lifecycle, races,
//! a Linux-style driver, and a reference model), declared in one place beside their shared rig.

// The suites below read the device's own items through `super::*`, as the older test files do.
#[allow(unused_imports)]
use super::*;

#[path = "atomicity_tests.rs"]
mod atomicity_tests;
#[path = "bounds_tests.rs"]
mod bounds_tests;
#[path = "functional_tests.rs"]
mod functional_tests;
#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;
#[path = "model_tests.rs"]
mod model_tests;
#[path = "race_tests.rs"]
mod race_tests;
#[path = "rig.rs"]
mod rig;
#[path = "wire_tests.rs"]
mod wire_tests;
