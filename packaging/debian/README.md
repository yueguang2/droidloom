# Debian and Ubuntu packaging

Ubuntu uses the same runtime/image payload as the Arch packages. `droidloom-package
-- deb` unpacks one matching pacman pair and writes `droidloom-runtime` and
`droidloom-image` `.deb` archives; see
[BUILDING.md](../../docs/BUILDING.md#debian-and-ubuntu-packages).

The maintainer scripts in [`scripts/`](scripts) replace the libalpm hooks used by
the Arch packages:

- `droidloom-runtime.preinst` stops a running cell before an upgrade replaces
  mounted Android images.
- `droidloom-runtime.postinst` refreshes systemd and transfers an existing
  package-managed setup to the new helper.
- `droidloom-runtime.prerm` stops the cell and removes exported desktop
  launchers before removal.
- `droidloom-image.preinst` also stops the cell for an image-only upgrade.

`preinst`, `postinst`, `prerm` and `postrm` run as root through `dpkg`; they call
the same `/usr/lib/droidloom/droidloom-package-helper` lifecycle operations used
by pacman. Android data and personal configuration are retained on removal.

The package dependencies are translated from the pacman `depend` entries in
`tools/droidloom-package/src/debian.rs`. If the Arch package gains a new
dependency, add its Ubuntu name there before shipping the `.deb` pair.
