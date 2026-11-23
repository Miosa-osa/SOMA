//! Check that delivered launch material names the instance and generation it was made for.

use super::Error;

pub(super) fn validate_identity(
    generation: [u8; 32],
    instance: [u8; 16],
    operation: [u8; 16],
) -> Result<(), Error> {
    if generation == [0; 32] || instance == [0; 16] || operation == [0; 16] {
        return Err(Error::InvalidBinding);
    }
    Ok(())
}
