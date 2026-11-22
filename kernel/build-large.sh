#!/usr/bin/env bash
#
# The large-shape guest kernel (compressed swap and transparent huge pages): kernel/build-variant.sh
# with the large fragment. Kept so existing callers and documents keep working.
#   kernel/build-large.sh [build|regen-config]
exec "$(dirname "${BASH_SOURCE[0]}")/build-variant.sh" large "$@"
