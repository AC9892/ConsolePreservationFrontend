# Changelog

## 0.1.0-alpha.2

- Limit automatic NAND, firmware, and SD snapshot history to the newest three per category. Keep manual snapshots and current storage, and log retired restore points. Included in the alpha.2 backend. Older release ZIPs are unchanged. See [SNAPSHOTS.md](SNAPSHOTS.md).

## 0.1.0-alpha

- Added independent public DSi executable and first run import screen.
- Added user supplied BIOS, firmware, and NAND checks; system sources stay in their selected locations, with a separate writable virtual NAND.
- Set the virtual DSi battery to full and charged.
- Kept authentic firmware boot and the existing DSi desktop shell.
- Kept the existing development application separate.
