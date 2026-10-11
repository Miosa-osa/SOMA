# The SOMA default sandbox image with a live browser in it: `node:22` (Debian bookworm) plus
# Chromium, the fonts a page needs to render, and nothing else.
#
# Build it for the guest architecture:
#
#   docker build --platform=linux/amd64 -t soma-browser-default:1 workloads/browser-default
#
# The base is pinned by index digest, the way `workloads/large-dax` and `kernel/Dockerfile` pin
# theirs, so the image names one reviewed `node:22` rather than whatever the tag points at the
# day it is built. Move the pin deliberately:
#
#   docker buildx imagetools inspect node:22 | head -3
ARG NODE_IMAGE=node@sha256:0e5f906573693feaa1e21057ebdcfdb5bd5021f050b2dc7c9deceb629c7da2a8
FROM ${NODE_IMAGE}

ENV DEBIAN_FRONTEND=noninteractive

# Why each package is here:
#
#   chromium                 the browser. Debian ships it with every shared library it links
#                            (nss, gbm, atk, cups, asound, xkbcommon, ...), so there is no
#                            second list of libraries to keep in step with the browser.
#   fonts-liberation         metric-compatible Arial, Times New Roman, and Courier New; most
#                            pages name one of them.
#   fonts-noto-core          Latin, Cyrillic, Greek, Arabic, Hebrew, Indic, and Thai scripts.
#   fonts-noto-cjk           Chinese, Japanese, and Korean. This is the largest single package
#                            and the reason fonts are measured separately in the design.
#   fonts-noto-color-emoji   emoji, which render as empty boxes otherwise.
#
# `--no-install-recommends` keeps Mesa's software stack and the desktop integration out; the
# browser runs headless and needs neither. The index lists are removed because nothing at
# capture time can use them (a Generation's warm plan has no network) and a sandbox that runs
# `apt-get update` fetches them fresh either way, exactly as it does on plain `node:22`.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      chromium \
      fonts-liberation \
      fonts-noto-cjk \
      fonts-noto-color-emoji \
      fonts-noto-core \
 && fc-cache -f \
 && apt-get clean \
 && find /var/lib/apt/lists -type f -delete \
 && chromium --version

# The page-cache warm list the guest agent reads before a Generation is captured (`/etc/soma/warm`,
# one absolute path per line). Reading a file populates the guest page cache, the capture records
# those pages, and every restored Instance starts with them already resident, so the first
# browser start in a fresh sandbox reads nothing from disk.
#
# Naming a list replaces the agent's conventional one, so the interpreters the conventional list
# would have warmed are named here too. The browser binary and the shared libraries it links come
# from `ldd` at build time rather than from a hand-kept list, so a Chromium upgrade cannot leave
# the list naming a library the new build no longer loads. The agent bounds a list at 256 entries
# and 512 MiB per file; this one is about 115 entries and 460 MiB in all.
#
# Only reading happens at capture. Nothing here starts a browser, so the snapshot holds no
# browser process, no profile, and no random state shared between Instances.
RUN mkdir -p /etc/soma \
 && { \
      echo /usr/lib/chromium/chromium; \
      echo /usr/lib/chromium/resources.pak; \
      echo /usr/lib/chromium/icudtl.dat; \
      echo /usr/lib/chromium/v8_context_snapshot.bin; \
      echo /usr/lib/chromium/chrome_100_percent.pak; \
      echo /usr/local/bin/node; \
      echo /usr/bin/python3; \
      ldd /usr/lib/chromium/chromium | awk '/=> \//{print $3}'; \
    } | awk '!seen[$0]++' > /etc/soma/warm \
 && test "$(wc -l < /etc/soma/warm)" -gt 50
