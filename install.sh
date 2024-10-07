#!/bin/sh
set -ev

sudo -v

cargo build --release

sudo systemctl stop lemond
sudo cp target/release/lemond /opt/lemond
sudo cp target/release/pre-suspend /opt/lemond-pre-suspend
sudo systemctl enable --now lemond

# systemctl --user restart lemond-client

systemctl --no-pager status lemond
journalctl -u lemond -f
