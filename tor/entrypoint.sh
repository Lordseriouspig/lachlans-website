#!/bin/sh

#
# Copyright (C) 2026 Lachlan
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU Affero General Public License as
# published by the Free Software Foundation, either version 3 of the
# License, or (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU Affero General Public License for more details.
#
# You should have received a copy of the GNU Affero General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#

set -eu

mkdir -p /var/lib/tor/lachlans-website
chown -R debian-tor:debian-tor /var/lib/tor
chmod 700 /var/lib/tor/key-files
chmod 600 /var/lib/tor/key-files/*

exec gosu debian-tor tor -f /etc/tor/torrc
