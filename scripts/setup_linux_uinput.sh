#!/usr/bin/env sh
set -eu

cat <<'EOF'
Deskbridge native Linux input uses two kernel interfaces:

  /dev/uinput          client-side virtual keyboard/mouse injection
  /dev/input/event*    server-side global keyboard/mouse capture

For Linux server X11 real cursor tracking and multi-monitor layout detection, install:

  sudo apt install -y libx11-dev x11-xserver-utils

Run these permission commands once, then log out and back in:

  sudo modprobe uinput
  sudo usermod -aG input "$USER"
  printf '%s\n' 'KERNEL=="uinput", MODE="0660", GROUP="input", OPTIONS+="static_node=uinput"' | sudo tee /etc/udev/rules.d/70-deskbridge-uinput.rules
  printf '%s\n' 'KERNEL=="event*", SUBSYSTEM=="input", MODE="0660", GROUP="input"' | sudo tee /etc/udev/rules.d/71-deskbridge-input-events.rules
  sudo udevadm control --reload-rules
  sudo udevadm trigger --subsystem-match=misc
  sudo udevadm trigger --subsystem-match=input

After logging back in, verify:

  test -w /dev/uinput && echo uinput-ok
  test -r /dev/input/event0 && echo input-events-readable

If your distribution does not have an input group, create a dedicated group and
replace GROUP="input" in both rules with that group name.
EOF
