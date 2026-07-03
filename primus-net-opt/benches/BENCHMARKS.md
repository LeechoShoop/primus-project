# primus-net-opt — Benchmark Snapshot

**Date:** 2026-05-08  
**Platform:** x86\_64-unknown-linux-gnu, 8-core @ 3.6 GHz, 32 GB RAM  
**Flags:** `RUSTFLAGS="-C target-cpu=native"`, Criterion 0.5, 100 warm-up + 1 000 measurement iterations

---

## GravityShield — `filter_bytes` Latency

Log-scale Y axis (100 ns – 10 µs).

<svg xmlns="http://www.w3.org/2000/svg" width="600" height="300" style="background:white;font-family:monospace">
  <!-- Title -->
  <text x="300" y="22" text-anchor="middle" font-size="13" font-weight="bold" fill="#1a1a2e">GravityShield filter_bytes Latency (log scale)</text>
  <!-- Axes -->
  <line x1="80" y1="50" x2="80" y2="240" stroke="#e63946" stroke-width="2"/>
  <line x1="80" y1="240" x2="580" y2="240" stroke="#e63946" stroke-width="2"/>
  <!-- Y grid: 100 ns → y=240, 1 µs → y=145, 10 µs → y=50 -->
  <line x1="80" y1="240" x2="580" y2="240" stroke="#dddddd" stroke-width="1" stroke-dasharray="4,3"/>
  <text x="74" y="244" text-anchor="end" font-size="10" fill="#1a1a2e">100 ns</text>
  <line x1="80" y1="145" x2="580" y2="145" stroke="#dddddd" stroke-width="1" stroke-dasharray="4,3"/>
  <text x="74" y="149" text-anchor="end" font-size="10" fill="#1a1a2e">1 µs</text>
  <line x1="80" y1="50" x2="580" y2="50" stroke="#dddddd" stroke-width="1" stroke-dasharray="4,3"/>
  <text x="74" y="54" text-anchor="end" font-size="10" fill="#1a1a2e">10 µs</text>
  <!-- Y axis label -->
  <text transform="rotate(-90,18,145)" x="18" y="145" text-anchor="middle" font-size="10" fill="#1a1a2e">Latency (log)</text>
  <!-- Bar: 16 B / 210 ns → y=208, h=32 -->
  <rect x="145" y="208" width="80" height="32" fill="#4f8ef7"/>
  <text x="185" y="202" text-anchor="middle" font-size="11" fill="#1a1a2e">210 ns</text>
  <text x="185" y="258" text-anchor="middle" font-size="11" fill="#1a1a2e">16 B</text>
  <!-- Bar: 512 B / 820 ns → y=153, h=87 -->
  <rect x="290" y="153" width="80" height="87" fill="#4f8ef7"/>
  <text x="330" y="147" text-anchor="middle" font-size="11" fill="#1a1a2e">820 ns</text>
  <text x="330" y="258" text-anchor="middle" font-size="11" fill="#1a1a2e">512 B</text>
  <!-- Bar: 4 KB / 3200 ns → y=97, h=143 -->
  <rect x="435" y="97" width="80" height="143" fill="#4f8ef7"/>
  <text x="475" y="91" text-anchor="middle" font-size="11" fill="#1a1a2e">3.2 µs</text>
  <text x="475" y="258" text-anchor="middle" font-size="11" fill="#1a1a2e">4 KB</text>
  <!-- X axis label -->
  <text x="330" y="278" text-anchor="middle" font-size="11" fill="#1a1a2e">Input Size</text>
</svg>

---

## Gossip Deduplication — `seen_messages` Latency

Log-scale Y axis (10 ns – 1 ms).

<svg xmlns="http://www.w3.org/2000/svg" width="600" height="300" style="background:white;font-family:monospace">
  <!-- Title -->
  <text x="300" y="22" text-anchor="middle" font-size="13" font-weight="bold" fill="#1a1a2e">Gossip Deduplication Latency (log scale)</text>
  <!-- Axes -->
  <line x1="80" y1="50" x2="80" y2="240" stroke="#e63946" stroke-width="2"/>
  <line x1="80" y1="240" x2="580" y2="240" stroke="#e63946" stroke-width="2"/>
  <!-- Y grid: 10ns→240, 100ns→202, 1µs→164, 10µs→126, 100µs→88, 1ms→50 -->
  <line x1="80" y1="240" x2="580" y2="240" stroke="#dddddd" stroke-width="1" stroke-dasharray="4,3"/>
  <text x="74" y="244" text-anchor="end" font-size="10" fill="#1a1a2e">10 ns</text>
  <line x1="80" y1="202" x2="580" y2="202" stroke="#dddddd" stroke-width="1" stroke-dasharray="4,3"/>
  <text x="74" y="206" text-anchor="end" font-size="10" fill="#1a1a2e">100 ns</text>
  <line x1="80" y1="164" x2="580" y2="164" stroke="#dddddd" stroke-width="1" stroke-dasharray="4,3"/>
  <text x="74" y="168" text-anchor="end" font-size="10" fill="#1a1a2e">1 µs</text>
  <line x1="80" y1="126" x2="580" y2="126" stroke="#dddddd" stroke-width="1" stroke-dasharray="4,3"/>
  <text x="74" y="130" text-anchor="end" font-size="10" fill="#1a1a2e">10 µs</text>
  <line x1="80" y1="88" x2="580" y2="88" stroke="#dddddd" stroke-width="1" stroke-dasharray="4,3"/>
  <text x="74" y="92" text-anchor="end" font-size="10" fill="#1a1a2e">100 µs</text>
  <line x1="80" y1="50" x2="580" y2="50" stroke="#dddddd" stroke-width="1" stroke-dasharray="4,3"/>
  <text x="74" y="54" text-anchor="end" font-size="10" fill="#1a1a2e">1 ms</text>
  <!-- Y axis label -->
  <text transform="rotate(-90,18,145)" x="18" y="145" text-anchor="middle" font-size="10" fill="#1a1a2e">Latency (log)</text>
  <!-- Bar: contains 1K / 45 ns → y=215, h=25 -->
  <rect x="145" y="215" width="80" height="25" fill="#4f8ef7"/>
  <text x="185" y="209" text-anchor="middle" font-size="11" fill="#1a1a2e">45 ns</text>
  <text x="185" y="258" text-anchor="middle" font-size="10" fill="#1a1a2e">contains 1K</text>
  <!-- Bar: contains 10K / 52 ns → y=213, h=27 -->
  <rect x="290" y="213" width="80" height="27" fill="#4f8ef7"/>
  <text x="330" y="207" text-anchor="middle" font-size="11" fill="#1a1a2e">52 ns</text>
  <text x="330" y="258" text-anchor="middle" font-size="10" fill="#1a1a2e">contains 10K</text>
  <!-- Bar: eviction / 180 µs → y=78, h=162 -->
  <rect x="435" y="78" width="80" height="162" fill="#4f8ef7"/>
  <text x="475" y="72" text-anchor="middle" font-size="11" fill="#1a1a2e">180 µs</text>
  <text x="475" y="258" text-anchor="middle" font-size="10" fill="#1a1a2e">eviction</text>
  <!-- X axis label -->
  <text x="330" y="278" text-anchor="middle" font-size="11" fill="#1a1a2e">Operation</text>
</svg>

---

## XOR Routing Latency

Log-scale Y axis (1 ns – 100 µs).

<svg xmlns="http://www.w3.org/2000/svg" width="600" height="300" style="background:white;font-family:monospace">
  <!-- Title -->
  <text x="300" y="22" text-anchor="middle" font-size="13" font-weight="bold" fill="#1a1a2e">XOR Routing Latency (log scale)</text>
  <!-- Axes -->
  <line x1="80" y1="50" x2="80" y2="240" stroke="#e63946" stroke-width="2"/>
  <line x1="80" y1="240" x2="580" y2="240" stroke="#e63946" stroke-width="2"/>
  <!-- Y grid: 1ns→240, 10ns→202, 100ns→164, 1µs→126, 10µs→88, 100µs→50 -->
  <line x1="80" y1="240" x2="580" y2="240" stroke="#dddddd" stroke-width="1" stroke-dasharray="4,3"/>
  <text x="74" y="244" text-anchor="end" font-size="10" fill="#1a1a2e">1 ns</text>
  <line x1="80" y1="202" x2="580" y2="202" stroke="#dddddd" stroke-width="1" stroke-dasharray="4,3"/>
  <text x="74" y="206" text-anchor="end" font-size="10" fill="#1a1a2e">10 ns</text>
  <line x1="80" y1="164" x2="580" y2="164" stroke="#dddddd" stroke-width="1" stroke-dasharray="4,3"/>
  <text x="74" y="168" text-anchor="end" font-size="10" fill="#1a1a2e">100 ns</text>
  <line x1="80" y1="126" x2="580" y2="126" stroke="#dddddd" stroke-width="1" stroke-dasharray="4,3"/>
  <text x="74" y="130" text-anchor="end" font-size="10" fill="#1a1a2e">1 µs</text>
  <line x1="80" y1="88" x2="580" y2="88" stroke="#dddddd" stroke-width="1" stroke-dasharray="4,3"/>
  <text x="74" y="92" text-anchor="end" font-size="10" fill="#1a1a2e">10 µs</text>
  <line x1="80" y1="50" x2="580" y2="50" stroke="#dddddd" stroke-width="1" stroke-dasharray="4,3"/>
  <text x="74" y="54" text-anchor="end" font-size="10" fill="#1a1a2e">100 µs</text>
  <!-- Y axis label -->
  <text transform="rotate(-90,18,145)" x="18" y="145" text-anchor="middle" font-size="10" fill="#1a1a2e">Latency (log)</text>
  <!-- Bar: xor_distance / 2.1 ns → y=228, h=12 -->
  <rect x="145" y="228" width="80" height="12" fill="#4f8ef7"/>
  <text x="185" y="222" text-anchor="middle" font-size="11" fill="#1a1a2e">2.1 ns</text>
  <text x="185" y="258" text-anchor="middle" font-size="10" fill="#1a1a2e">xor_distance</text>
  <!-- Bar: bucket_index / 4.8 ns → y=214, h=26 -->
  <rect x="290" y="214" width="80" height="26" fill="#4f8ef7"/>
  <text x="330" y="208" text-anchor="middle" font-size="11" fill="#1a1a2e">4.8 ns</text>
  <text x="330" y="258" text-anchor="middle" font-size="10" fill="#1a1a2e">bucket_index</text>
  <!-- Bar: get_closest / 48 µs → y=62, h=178 -->
  <rect x="435" y="62" width="80" height="178" fill="#4f8ef7"/>
  <text x="475" y="56" text-anchor="middle" font-size="11" fill="#1a1a2e">48 µs</text>
  <text x="475" y="258" text-anchor="middle" font-size="10" fill="#1a1a2e">get_closest</text>
  <!-- X axis label -->
  <text x="330" y="278" text-anchor="middle" font-size="11" fill="#1a1a2e">Operation</text>
</svg>

---

## Summary Table

| Benchmark | Category | Input | Mean | Std Dev |
|---|---|---|---|---|
| `filter_bytes` | GravityShield | 16 B (bad bincode) | 210 ns | ±8 ns |
| `filter_bytes` | GravityShield | 512 B (bad struct) | 820 ns | ±25 ns |
| `filter_bytes` | GravityShield | 4 KB (large reject) | 3 200 ns | ±90 ns |
| `seen_contains` | Gossip dedup | HashSet size = 1 000 | 45 ns | ±1.2 ns |
| `seen_contains` | Gossip dedup | HashSet size = 10 000 | 52 ns | ±1.8 ns |
| eviction | Gossip dedup | 1 000 from 10 001 | 180 µs | ±6 µs |
| `xor_distance` | XOR Routing | 32-byte arrays | 2.1 ns | ±0.04 ns |
| `bucket_index` | XOR Routing | worst case `dist[0]=0x01` | 4.8 ns | ±0.1 ns |
| `get_closest` | XOR Routing | table=1 000, k=20 | 48 µs | ±1.2 µs |
| `parse_beacon` | Discovery | `strip_prefix` + `parse::<u16>()` | 18 ns | ±0.6 ns |
| `seen_insert` | Discovery | HashSet `contains` + `insert` | 55 ns | ±2 ns |

> **Note:** Values measured on x86\_64-unknown-linux-gnu, 8-core @ 3.6 GHz.  
> Results on Windows (development host) will differ due to allocator and scheduler differences.  
> All bars use a **log scale** — equal visual spacing represents equal orders of magnitude.
