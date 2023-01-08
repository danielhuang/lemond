#!/bin/sh
set -ev

sudo systemctl disable --now lemond
cargo build
sudo target/debug/lemond
