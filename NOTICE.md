# Notices

RF-Rig is an independent RackForge plugin implemented in Rust and distributed
under GPL-3.0-only.

The project models classic guitar effect *circuits* — a transconductance
compressor, an op-amp overdrive with feedback clipping, a hard-clipping
distortion, a two-stage fuzz, a bucket-brigade chorus and echo, a spring tank
and a plate — from publicly documented topologies and component values. Circuit
topologies are not copyrightable subject matter, and the patents covering these
designs expired decades ago.

RF-Rig is not affiliated with or endorsed by any manufacturer. No third-party
firmware, artwork, product photograph, trademark or brand name is included in
this repository or in the plugin package, and none is used to describe the
pedals: each one is named for what it does and documented by the circuit family
it derives from.

Third-party code: the plugin depends on `rackforge-plugin-sdk` (GPL-2.0-or-later
as part of RackForge) and on `libm` (MIT/Apache-2.0).
