#!/bin/sh
set -ev

sudo -v

cargo build --release
cargo build --release --bin client

sudo systemctl stop lemond
sudo cp target/release/lemond /opt/lemond
sudo systemctl enable --now lemond

# systemctl --user restart lemond-client

systemctl --no-pager status lemond
journalctl -u lemond -f
