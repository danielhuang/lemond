#!/bin/sh

sudo systemctl stop lemond
cargo build
sudo flamegraph -F 40000 target/debug/lemond
sudo chown "$USER" -- *
