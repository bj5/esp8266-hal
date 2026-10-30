# WiFi 证据

`src/wifi` 用 `no_std` Rust 重新实现闭源 `ESP8266_NONOS_SDK` 的原始帧（raw-frame）子集。移植里的每一次
寄存器写、每个字段偏移、每段控制流都来自对 SDK 归档的反编译，可以验证的部分都在硬件上跑过（ESP8266EX、
26 MHz 晶振、4 MB flash、MAC `84:f3:eb:b4:8c:34`）。本文记录每条结论的来源，以及哪些部分还无法确定。

## 方法

- SDK 从 `https://github.com/espressif/ESP8266_NONOS_SDK` 克隆。REA 打不开 `.a` 归档，所以成员用 `ar x`
  解出（`net8211`、`main`、`phy`、`pp`）。
- 分析用 Ghidra headless，语言选 `Xtensa:LE:32:default`，配合 `xtensa-esp-elf-objdump`。Ghidra 对若干函数
  失败（bad instruction data），所以 `ram_pbus_set_rxgain`、`gen_rx_gain_table`、`set_crystal_uart`、
  `rc_cal`、`init_cal_dcoffset`、`chip_v6_set_chan`、`chip_60_set_channel` 只能靠 objdump 看。
- 没有 ESP8266 ROM 镜像，ROM 条目无法反编译。它们的行为由 `phy_chip_v6.o`（即 `libphy` 自己的
  `.irom0.text` 副本）里的调用点，以及在板子上逐个调用槽位来确定。
- 硬件事实来自烧进板子的 example 通过 UART0（115200）打印的输出，外加 `esptool read-flash`。烧写必须用
  `--flash-mode dio`：这块板子的 QIO 数据读路径返回全 0。
- 黑盒硬件测量由一次性的探针 example 完成，结论写进本文后探针就删除。I2C 位字段网格的原始抓取和拟合
  脚本在仓库外的 `/tmp/opencode`：`i2c_probe.txt`、`i2c_probe2.txt`、`i2c_probe3.txt`、`i2c_check.txt`、
  `wifi_scan_inst.txt`、`wifi_rx_selftest3.txt`、`wifi_rx_selftest4.txt`、`ghidra_script/fit_i2c.py`。

## ROM 函数表

`phy_get_romfuncs()` 返回一张 68 项的表，在这块板子上表基址是 `0x3fffc2f8`。槽位名字是按地址匹配
`ld/eagle.rom.addr.v6.ld` 里按地址排列的 `PROVIDE`，不是按索引匹配。`phy_get_romfunc_addr @0x00013894` 会把
其中 13 个槽换成 libphy 自己的实现（9、10、18、22、25、29、30、31、34、40、54、58、67），所以 SDK 在这些
位置跑 `ram_*` 代码，而移植调用的是 ROM 条目。槽位之间的区域被当数据用（`register_chipv6_phy` 写
`g_phyFuns[3]`、`g_phyFuns[0x1b]`、`g_phyFuns[0x32]`），所以这些单元必须按字节读。

槽 5、23、24 是空槽，因此 6..22 的名字索引 = 表索引 - 1，25 往后是表索引 - 3。值得注意的是没有
`phy_init` 的 `PROVIDE`，所以槽 0 确实是 `rom_abs_temp`。

| 槽 | 偏移 | 名字 | 由调用点确立的参数个数 | 硬件结论 |
|---|---|---|---|---|
| 0 | 0x00 | `rom_abs_temp` | 2（`register_phy_ops` 用 `(1, 0)` 调槽 0） | 返回 |
| 1 | 0x04 | `rom_chip_v6_set_chanfreq` | 0；接收 **MHz**，所以 `phy::set_channel` 换算 `channel -> 2412 + 5 * (channel - 1)` | 返回 |
| 2 | 0x08 | `rom_chip_v5_enable_cca` | 0 | 返回 |
| 3 | 0x0c | `rom_chip_v5_sense_backoff` | 0 | 返回 |
| 4 | 0x10 | `rom_dc_iq_est` | 0 | **故障**（`rst cause:4`），AGC 路径不可用 |
| 5 | 0x14 | - | - | 空槽 |
| 6 | 0x18 | `rom_en_pwdet` | 0 | 返回 |
| 7 | 0x1c | `rom_get_bb_atten` | 0 | 返回 `0x7f` |
| 8, 9 | 0x20, 0x24 | `rom_get_fm_sar_dout` | 0 | **从不返回** |
| 10 | 0x28 | `rom_get_noisefloor` | 0 | 返回 |
| 14 | 0x38 | `rom_linear_to_db` | 0 | param 块为空时返回 0 |
| 17 | 0x44 | `rom_mhz2ieee` | 2 | 返回；`mhz2ieee` 表在 `phy` 里重建 |
| 32 | 0x80 | `rom_set_channel_freq` | 2 | 返回，但移植不用它（见下文 RF 调谐） |
| 34 | 0x88 | `rom_chip_v5_rx_init` | 0 | 返回 |
| 36, 37, 38, 39 | 0x90..0x9c | i2c 读写辅助 | 37 取 **5** 个参数（`rom_i2c_readReg_Mask`），后两个是位字段的高位、低位索引（见"I2C 的 cell 与位字段"） | 返回 |
| 40, 47 | 0xa0, 0xbc | pbus debug / work mode | 0 | 返回 |
| 43, 44, 45 | 0xac, 0xb0, 0xb4 | `rom_pbus_force_test`、`rom_pbus_rd`、`rom_pbus_set_rxgain` | 44 取 **2** 个参数（`ram_pbus_set_rxgain`） | 返回 |
| 49, 50 | 0xc4, 0xc8 | `rom_pbus_xpd_rx_on`、`rom_pbus_xpd_tx_off` | 0 | 返回 |
| 54, 55, 56, 57 | 0xd8..0xe4 | `rom_rfpll_*`、`rom_write_rfpll_sdm` | 54 取 0、55 取 **1**、56 取 **1**、57 取 **4** | 都按反汇编确立的参数个数返回 |

参数个数那一列很关键：第一次探针遍用 0 个参数调用每个槽，对槽 37、54、55、56、57 来说那是假象。

## 控制块同时也是数据

Ghidra 的 `.bss`/`.data` 块（`0x10000-0x15100`，包含 `wDevCtrl`、`lmacConfMib` 和 `esf_buf` 池）对应
DRAM `0x3FFE0000-0x3FFE5F40`，也就是 `system_print_meminfo` 报告堆起始的位置。Ghidra 里的字段名是**十进制**
偏移，而 `annot.py` 的字面量标签是**十六进制**加数；以 `0x10000` 为基址两者可以互换，例如 `0x15018` =
十进制 24 = 模板描述符。

## 描述符与 18 位 desc 窗口

`lldesc` 是一条 3 字的记录（control、buffer、next）；布局来自 `wdev.o` 实际使用的偏移，不是 SDK 头文件。
`wDev_Initialize @0x00010934` 用 ROM 辅助函数 `lldesc_build_chain(0x48, ...)` 建链——每 buffer `0x2598`、
每 info block `0x644`、6 个描述符——然后重写每个描述符：

```c
*puVar1 = *puVar1 & 0xfffff000 | (*puVar1 & 0xfff) - 4 & 0xfff;   /* size - 4 放在 bit 0..11 */
*puVar1 = *puVar1 & 0xff000fff | (*puVar1 & 0xfff) << 0xc;        /* 在 bit 12..23 镜像一份 */
*puVar2 = 0xdeadbeef;                                             /* buffer 起始的 canary */
*(undefined4 *)((int)puVar2 + (*puVar1 & 0xfff)) = 0xdeadbeef;    /* buffer 结尾的 canary */
```

`wDev_AppendRxBlocks @0x0001112c` 交回一个描述符时：置 bit 31，清 bit 30 和 28，重新套用 size 镜像。移植在
Rust 里自己建链，不调用 ROM 辅助函数。

`0x3ff20080/84/88` 存的是三个 `lldesc_build_chain` 的 desc 空间基址（`&DAT_00014c3c`、`&DAT_00012650`、
`&DAT_00012650`），不是链头；链头在 `wDevCtrl + 8/40/48`。对于连续的链，第一个描述符地址等于 desc 空间基址，
这就是 `mac::init_ring` 把 `desc_base` 写进 `REG_CHAIN_A` 的原因。链掩码（`0x3ff20ca8 |= 0x10000000`、
`0x3ff20cac |= 0x50000000`）只在描述符数量大于 1 时生效。这些寄存器由闭源的 ROM 辅助函数写，没有任何反编译
出来的 `wdev.o` 函数写它们。

两条硬件事实限定了环可以放在哪里：

- SDK 的 desc 空间 `0x3FFE0000-0x3FFE5F40` 在这块板子上**不是 CPU 可访问的 RAM**：写
  `0x3ffe1000`、`0x3ffe2650`、`0x3ffe4c3c`、`0x3ffe5f40` 都读回 0。Rust 建的链只能放在 `dram_seg`
  （`0x3FFE8000` 起），移植就是这么做的。
- `lmacTxFrame` 把描述符地址掩到 18 位（`*(uint *)(param_1 + 4) & 0x3ffff`），所以硬件通过一个 256 KB 窗口
  解析描述符。一次 TX 会让 MAC 读到移植的描述符 `0x3ffea9d4`（窗口偏移 `0x2a9d4`），并读回写入的原值
  （slot + 4 处是 `0x2b22a9d4`），所以 MAC 确实看得见移植选定的 DRAM 地址。这就排除了"环的位置"是 RX 的障碍。

## FIQ

`wDev_ProcessFiq @0x00011568` 在 `app_main` 的 `FUN_00010b00` 里用 `ets_isr_attach(0, &wDev_ProcessFiq)` +
`ets_isr_unmask(1)` 挂上，所以 HAL 按 `InterruptType::FIQ` 分发它。

三个寄存器由处理程序本身确立：`0x3ff20c18` 是**使能掩码**（处理程序在每条退出路径上保存它、清除它、再恢复
它），`0x3ff20c20` 是状态，`0x3ff20c24` 是镜像。这块板子上的掩码是 `0x2c9f0300` = bit 8、9、16、17、18、19、20、
23、26、27、29；`wdev_go_sniffer` 会加上 `0xc`。状态位可以不受掩码影响而置起（bit 2、3、28 有分支但不在掩码
里）。

`wDev_ProcessFiq` 里的状态位含义：

| bit | 反编译中的含义 |
|---|---|
| `0x10000000`（bit 28） | 不可恢复错误：`ets_printf("wdev.c 1162")` 加死循环 |
| `0x8000000`、`0x4000000`（bit 27、26） | 可选回调 `DAT_00011ec0` / `DAT_00011ec4`（`wDev_MacTim1SetFunc`） |
| `(x & 0xc) == 8`（bit 3） | sniffer / pp 分支 |
| `(x & 0x104) == 4`（bit 2） | 硬件加密分支 |
| `0x100`（bit 8） | RX 描述符完成，要求 `wDevCtrl[5] != 0` |
| `0xf0000` 且 `0x80000`（bit 19） | TX 状态：分支要求 bit 19 置起，所以 bit 16、17、18 单独出现时不进入它 |
| `0x20000000`（bit 29） | all-TX-timeout |
| `0x100000`（bit 20） | 冲突 |
| `0x200`（bit 9）、`0x800000`（bit 23） | 在使能掩码里，但处理程序**没有对应分支**：只有这些位置起时 FIQ 照样触发，处理程序确认状态后直接返回 |

硬件直方图（10 次 TX、处理程序进入 30 次、状态按位或 `0x00890000`）：每次发送的图案是 bit 16
（`0x10000`）、然后 bit 19（`0x80000`，TX 状态分支）、然后 bit 27（`0x8000000`，回调）。bit 8 从不触发，没有
任何描述符被标记完成（`owned 24`、`current desc 0`、`rx_ok 0`）。所以 RX 的**软件**路径和 FIQ 路径是通的，
不产出东西的是 RX 数据路径。

被动遍历（不发帧，只是信道 1..14 逐信道驻留）的那次插桩运行给出的数字是：init 后掩码读 `0x2c9f0300`、事件
前状态读 `0x00000000`、每个信道的 RF PLL 都**没有**锁定、处理程序进入 3 次（在第 1、2、4 信道）、三次状态都
恰好是 `0x00800000`，也就是只有 bit 23；`rx_ok 0`、`rx_error 0`、`fiq_errors 0`、`owned 24`。也就是说 MAC 在
被动扫描里确实会拉 FIQ，但拉的是"使能却没有分支"的 bit 23；RX 完成所需的 bit 8 一次都没有出现。bit 23 的
含义没有任何证据支持：`0x3ff20c20` 在 SDK 里只被处理程序读，被硬件写，而反编译出的代码里没有对它的 bit 23
的分支，所以这块板子上唯一能说的是它由硬件置起且被忽略。插桩探针的原始抓取是
`/tmp/opencode/wifi_scan_inst.txt`，探针 example 在结论记录后删除。

软件不能注入 FIQ：对 `0x3ff20c20` 和 `0x3ff20c24` 的写不落地（写 `0x100` 后读回 `0x00000000`，处理程序进入
次数不变），而对使能掩码 `0x3ff20c18` 的写会落地（测试写 `| 0x1` 读回 `0x2c9f030d`，随即恢复）。所以
RX-complete 只能由 MAC 置起，描述符那一半无法用软件自测。扫描期间掩码读回 `0x2c9f030c`，即基础掩码加
`wdev_go_sniffer` 的 `0xc`，与反编译一致。

驻留时间对得上程序值：14 个信道 x 150 ms 的遍历从扫描开始到结束实测 2.2 s（每信道约 157 ms，多出的部分是
有界的 cal 轮询和打印）；`timer1.delay_ms(500)` 在 blinky 上实测平均约 500.5 ms。所以 ROM 的延时是准的，
WiFi 路径里那些比程序值长的观察不能归到 `delay_ms` 本身——那条路径的时间来自 cal 轮询窗口和 `bbpll_cal`
改动的时钟控制。

## RX 增益与 DC 校准

`gen_rx_gain_table`（`phy_chip_v6.o @0x00010524`）是纯算术，因此可移植：char 表是 blob 的字节 2..17
（`register_chipv6_phy_init_param` 里 `rx_gain_swp_step = param_1 + 2`，由 `phy_bb_rx_cfg` 作为
`g_phyFuns[0x33]` 传入，长度 `0x10`），u16 表是 `.rodata + 0x30`（`0, 4, 32, 36, 40, 44, 48, 52, 64, 96, 100,
104, 112, 116, 120, 124`），没有 ROM 调用，也没有 flash 校准数据。每个条目是
`u16_table[index] + ((1 << q) - 1) * 8 + column`，其中 `q = base / 6`、`column = base % 6`（上限 5）、
`base = row - accumulator`，按 16 位半字两两打包存进字数组（偶数行在低半、奇数行在高半）。行数是
`min(127, sign_extend_byte(rows))`，`phy_bb_rx_cfg` 传 127。生成器在 `q == 15` 且 `column == 5` 的那一行终止，
默认 blob 下是第 89 行；`phy_bb_rx_cfg` 把这个值作为 `DAT_000101b0 - 0xf` 写到 `0x60009b48`，移植现在会计算
并写入这个值。该寄存器**确实**接受写入：写 `0x4a` 后读回 `0x4a`，I2C 门关闭和打开两种情况都是。之前某次运行
在那儿读到的 `0x59`（= 89）是 `param::build` 留下的值，不是被拒的写，所以生成器的行遍历得到硬件佐证。

`ram_pbus_set_rxgain`（`phy_chip_v6_ana.o @0x4`）同样是纯算术加 ROM 调用，所以逐行照 objdump 转录：它用
`rom_pbus_rd` 读 pbus 块 3 和 2，保留 ROM 的掩码（块 3 是 `0x1c7`，块 2 是 `0x185`），再插入代码的位，其中
code bit 3..9 到 pbus 位置 0..6 是反转映射。转录本身在硬件上可核对，但只在 **debug mode** 下：work mode 里
每次 `rom_pbus_rd` 都报 `0x0000`（`/tmp/opencode/pbus_check.txt`），debug mode 里 `pbus_set_rxgain(0x18)` 之前块 3
读 `0x6`、块 2 读 `0x78`，写之后块 3 读回 `0x60`（就是 `pbus_set_rxbbgain` 对 code `0x18` 算出的值），块 2 读 `0`
（也正是转录为 code `0x18` 算出的值）——见 `/tmp/opencode/rx_probe_bankselect.txt`。也就是说 bus 上的值可读回，
它驱动的模拟锁存不可读回（下面 RX 一节）。另外 I2C 读路径是另一条可观察的写：设备 `0x77` 的 cell 3 在
debug mode 下写入 `0x1a` 后读回 `0x1a`。

反编译确立的 `phy_bb_rx_cfg` 增益操作顺序是：`rom_pbus_debugmode` -> `rom_pbus_force_test(1, 1, 0xc)` ->
条件分支 `(2, 1, 0x184)` 和 `(3, 2, 6)` -> `rom_pbus_workmode` -> `gen_rx_gain_table`（受 `piVar1[2]`，即控制块
偏移 8 那个 `.data` 字节保护）-> `set_rx_gain_testchip_50` -> 最后一个 I2C 写。`phy_bb_rx_cfg` 在调用生成器前
把 `DAT_00014e30`（= `.rodata + 0x30`）的 `0x20` 字节复制到栈缓冲，这确认 u16 表是 16 项。

`set_rx_gain_testchip_50`（`phy_chip_v6_cal.o @0x748`）用同样的方式解码生成的条目（bit 3..9 的 popcount 上限
4 = row，`entry & 7` = column，`index = row * 6 + column` 夹到 `0x1d`，这就是 `DC_TABLE_LEN = 30` 的由来），然后
要么测量要么复制。测量那一半需要 `rom_pbus_dco___SA2`（槽 59）和 AGC 条目 `rom_dc_iq_est`（槽 4），后者在这块
板子上故障；SDK 以 `*(ushort *)(row * 2 + param_4)` 读的每行表（`DAT_000101fa`，与生成器那张不是同一张）不存在，
因为校准扇区已擦除。`gen_rx_gain_table` 的 `(row >> 1) * 4 + param_1` 与 `set_rx_gain_testchip_50` 的
`row * 8 + param_3` 看似矛盾，其实不是：生成器写的是字数组的两个半，消费者按奇偶读同一数组
（`param_6 + (i >> 1)`），并把它的条目写到 `param_6 + 0x100` 处另一张 30 x 8 字节的表。

`phy::apply_rx_gain` 是那个消费者的可移植一半，并按反编译给它的位置执行：打开 I2C 门（`|= 0x30000`）、把
bank select 置 `0x1e0`、重新进入 pbus debug mode、保存两个位字段值、对反编译选中的偶数索引执行
`pbus_set_rxgain(entry & 0xfff)`，然后关门、用保存的值恢复两个字段，并按 `xpd_tx_off`、`xpd_rx_on`、
`workmode` 的顺序调用。测量那一半和写窗口那一半（`FUN_00010c2c`）没有复现。

用默认 blob 对这条路径做硬件检查：控制块字节 8 读 `0xEC`（所以守卫走的是 SDK 走的同一分支），生成器停在第
89 行，`apply_rx_gain` 选出 12 个偶数索引、应用 12 个增益码，序列结束后 I2C 门读 `0xff000104`（所以
`0x30000` 那些位确实开和关），bank select 读 `0x1e0`。pbus 读回只在 debug mode 有值（work mode 全是 `0x0000`），
基线是 `3/1 0x38`、`3/2 0x6`，强制写后六个 cell 全部读回写入的值，但 ADC 字段仍是 `0xe98`（块 4/5 的 DC cell
读回 `0x100` 时也不动，`/tmp/opencode/gain_probe.txt`）。因此可观察的是寄存器序列和 bus 上的值，不是它驱动的模拟锁存。
通过 `Wifi::init` 跑这条路径能走完整个扫描遍历而不故障。

增益窗口本身在这块板子上是**只读**的：`0x60009a68` 的 bank select 可写（`0x1e` 和 `0x1e0` 都能读回），但对
`0x60009e00 + row * 4` 的写在两种 select 下都不落地，窗口每一行都读常数 `0xf05a55aa`。用
`set_rx_gain_testchip_50` 的公式解码，这个值意味着 `dc1_high 449, dc1_low 210, row_base & 0x7ff 1386,
dc0 & 1 1`。code `0x18`（索引 12）实测的 DC 值是 `dc0 256, dc1 251`。select 1 的 pbus 读报的是模拟状态而不是
force 锁存，所以一次写只能通过读它写过的同一个 `(block, select)` 来验证。

## I2C 的 cell 与位字段

`rom_i2c_readReg(block, reg, offset)` 读一个字节——即某个 I2C 设备的一个 *cell*——而
`rom_i2c_readReg_Mask(block, reg, offset, high, low)` 读的是同一个 cell 里的一个位字段。`high` 是字段最高位的
索引，`low` 是最低位的索引：

```
result = (cell >> low) & ((1 << (high - low + 1)) - 1)
```

移位计数按 mod 32 取，这正是那些边界情况能自洽的原因：宽度 32 得到零字段，负宽度变成一个很大的掩码，也就是
整个 cell 右移。所以 `mask` **不是** AND 位掩码——之前的读法之所以看起来矛盾就是这个原因。

模型拟合的是在板子上测的 120 点网格：三个 cell，先用 `readReg` 得到它们的值，再对每个 cell 跑一遍 mask x
shift 网格（`/tmp/opencode/ghidra_script/fit_i2c.py`）。所有把 `mask` 当位掩码的候选只能拟合 120 点中的
40-85 个；位字段模型拟合全部 120 点。对 cell `0xe8` 它一次解释两个奇怪读数——`mask 0x08` 返回整个 cell，因为
字段 `[0,8]` 覆盖它；`mask 0xff` 返回 0，因为宽度 256 绕回 0。之后又在同样三个 cell 上做了 24 个未参与拟合的
`(high, low)` 组合的抽样检验（包含宽度 mod 32 绕回的情况），每个实测值都与预测一致
（`/tmp/opencode/i2c_check.txt`）。

按这种读法，SDK 的调用点就没有歧义了：

- `wait_rfpll_cal_end` 轮询的是 cell `98.1.7` 的字段 `[7,7]`，即校准字节的 bit 7。这块板子上该 cell 读
  `0xe8`，字段读 `1`，就是锁定位；同一 cell 的字段 `[0,7]` 返回整个字节，也就是 `readReg` 报的值。
- `set_rx_gain_testchip_50` 保存的是 cell `0x77.0.0x12` 的字段 `[0,7]`（整个字节）和 cell `0x77.0.0x18` 的字段
  `[5,5]`（单个位），然后在退出时用保存的值通过 `rom_i2c_writeReg_Mask` 写回，而该函数的最后一个参数是值。
  这就确认了移植使用的 `(block, reg, offset, high, low, value)` 顺序。

探针各轮得出的访问条件：I2C 门关闭时每个 cell 都读 0；用 `|= 0x30000` 打开门就足够访问校准设备 98，它的
cell 读出不同值。设备 106 在每一轮都读 0，而每一轮都在进入 `rom_pbus_debugmode` 之前，所以它的 cell 无法和
"读不到"区分开。设备 `0x77` 的每个 cell 都读 0，直到在 debug mode 下往 cell 3 写 `0x1a` 并读回，所以它的 cell
在 debug mode 下可以通过 i2c 路径读，但本身没有数据。索引大于 7 的 cell（`set_rx_gain_testchip_50` 用的那些）
没有直接读过，所以设备 `0x77` 的 cell 索引范围没有硬件依据。

## RF 调谐

RX 信道调谐就是 `set_rf_freq_offset` 序列（槽 55、57、56、54 加一次有界的 cal 轮询）加 `bbpll_cal`；
`rom_set_channel_freq`（槽 32）不用。`wait_rfpll_cal_end`（`phy_chip_v6_ana.o @0x90c`）最多轮询 `cal_locked`
100 次，每次 `ets_delay_us(20)`。之前在同一时刻看起来矛盾的地方（`readReg(98, 1, 1) = 0x80` 而
`readReg_Mask(98, 1, 7, 7, 7) = 0`）被上面的位字段模型解释掉了：两个调用读的是设备 98 的不同 cell ——
`0x80` 是 cell 1，而那个字段读是 cell 7（`0xe8`）的 `[7,7]`，即 bit 7，PLL 锁定前是 0、锁定后是 1。只有在
"两个调用访问同一个寄存器"的读法下它们才显得矛盾。也就是说早先那句"锁定出现在第 184 次轮询"是那次探针把
cell 98.1.1 的常数 `0x80` 当成了锁定标志；真正被轮询的字段在下面修正。

`set_cca`、`start_dig_rx`、`stop_dig_rx`、`dig_11b_filt_sel` 在硬件上验证过，`phy_bb_rx_cfg` 和 `bb_init` 的写
也验证过：`0x3ff20c70 = 0x240002`、`0x60000590 = 0x10`、`0x60009c04 = 0x1000`、`0x60009d1c = 0xfff`、
`0x60009860 = 1`、`0x60009b08 = 0x400`、`0x60009d68 = 0x415ce`。噪声门限条件
`(0x60009b60 >> 0xf & 5) == 5` 成立（`0x60009b60 = 0x69510`），而 `read_hw_noisefloor` 报 -180，缓存值是 -45，
也就是说 RX 前端已使能但不交付任何样本。`0x60009824` 是 DC 偏移锁存，不是信号强度寄存器，所以在没有外部信号
源的情况下没法把"房间里没有 RF"和"RX 数据路径是死的"区分开；同信道 TX 自回环也没有产生任何完成。

## RX 里软件可修的那部分

`src/wifi/noise.rs` 移植的是 RX 前端里不需要缺失数据的那半：`ram_set_noise_floor @0x00011838`、
`ram_start_noisefloor @0x00011898`、`read_hw_noisefloor @0x000118e4`、`ram_get_noisefloor @0x000117e0`、
`get_noisefloor_sat @0x00011800`、`do_noisefloor @0x00010d9c`、`noise_check_loop @0x0001191c`、
`phy_set_rx11b_reg @0x000141c0`——全是算术加寄存器写。三处此前缺失的写：

1. `0x60009a34`（模拟 RX 使能）。`noise_init @0x00011ab8` 的无校准分支写
   `(_DAT_60009a34 & 0xfffffc00 | 0xc9) & 0xfffffffe`，而 `phy_bb_rx_cfg` 在它之前把整个寄存器清 0，所以移植
   一直读到 0。现在 `NoiseTracker::seed` 执行这一笔，硬件读回 `0xc8`。
2. `0x60009b64` 的低 9 位（比较器门限）。`set_noise_floor(-388)` 写 `(quantized >> 1) + 0x200 = 0x13f`，
   硬件读回 `0x13f`——这是移植的算术第一次在这块板子上被读回证实；之前读到的 `0x1a6` 是 ROM 按每芯片校准值
   留下的。注意 `noise_check_loop` 结束时用掩码 `0xfffff000` 重写这 12 位（`start_noisefloor`），所以每轮之后
   可观察的门限是测量的基值 `0x1a0`，不是跟踪器写的那个值。
3. `0x60009820`（`phy_set_rx11b_reg`）。反汇编里没有它的调用点，所以按默认 blob 选中的 mode 0 显式调用。

`noise_init` 在测量循环里对 `0x60009a2c` bit 0 和 `0x60009d20` bit 30 的清除是**瞬态**：它先保存这两个寄存器，
走完信道 1、6、11、14，再恢复，所以净效果为零，`seed` 不做这两笔；它的测量循环需要 `noise_init` 的调用参数
（校准表），所以也不属于 `seed`。`noise_init` 还会置 `chip6_sleep_params` 的 bit 21（`0x200000`），即它进入时
测试的"已初始化"标志。

`noise_check_loop` 的两个参数由调用点确立：`pp.o @0x346` 和 `pm.o @0x29e` 都传 `a2 = 1`（转发给
`start_noisefloor` 的 seed），第二个参数是"是否把平均值写回比较器"的标志——`pp.o` 传 1，`pm.o` 的空闲路径传 0。
平均的方向由反汇编决定，不是对称的：样本高于门限时是 `sample + floor * 3`（75% 门限，噪声上升慢），低于时是
`floor + sample * 3`（75% 样本，下降快），取整是"和减 2（非负）或加 1（负）再 `>> 2`"。跟踪器的状态单元
（`piVar6[0x19]`、`piVar6 + 0x3a`、`DAT_000102e0`、`&g_phyFuns + 0xc5`）都在 88 字节控制块之外，所以状态由
`NoiseTracker` 持有；`func_0x00011988(&DAT_00010238, &DAT_00010241, piVar6[0x1b])` 未识别，跳过。

顺序由反汇编确立：`chip_v6_initialize_bb` 在 `0x2e9b` 调 `phy_bb_rx_cfg`，`0x2ea3` 写
`0x3ff20c70 |= 2`，`0x2ec2` 调 `func_0x000131ac`（TX 功率控制，未移植），`0x2ec4` 调 `noise_init`，
`0x2eca` 调 `tx_pwctrl_bg_init`。移植按 `rx_baseband` -> `0x3ff20c70 |= 2` -> `seed` 的顺序执行，并且每个
扫描驻留片调用一次 `check`（`Scan::poll`）。

硬件结果（`/tmp/opencode/rx_noise_probe_tx.txt`，探针 example 在记录后删除）：

```
before bring-up:  0x60009a34 = 0x00000000  0x60009b64 = 0xe9822fa6  0x60009d68 = 0x000015ce
after bring-up:   0x60009a34 = 0x000000c8  0x60009b64 = 0xe98b4f3f  0x60009d68 = 0x000415ce
                  0x60009820 = 0x00000008  0x60009c04 = 0x00001000  0x60009a2c = 0x00000000
                  adc 0xe98  threshold 0x13f  cca 0xb4  measuring true  ready false
```

按信道跑一轮跟踪器后，ADC 不再读常数：信道 2 到 14 报 -192、-192、-192、-192、-200、-193、-241、-236、-259、
-260、-253、-266、-262 dB，且 14 个信道里有 4 个在两次读之间值发生变化（之前 6 次以上采样在所有信道上都是
`0xe98`）。跟踪器的状态也确实被更新：结束时 floor -342、fast -346，即从种子 -388 向样本方向移动过。也就是说
模拟 RX 使能那一笔写下去之后 ADC **确实在转换**，并给出按信道不同的噪声底——这是 RX 前端活着的证据。

但 RX 数据路径仍然不交付描述符：信道 6 上 2 秒驻留、每片一次 `check`，`frames 0`、`rx_ok 0`，环上 24 个描述符
全部仍归 MAC 持有（`ring owned 24 of 24`），没有一个完成。FIQ 只在每次发送后按 `0x10000`、`0x80000` 交替
（TX 路径，与之前的结论一致），RX 完成位 `0x100` 从不出现。TX 自回环也测不到能量：空闲时 ADC 在 3572..3574
之间变化（跨度 2），20 次发送期间读数跨度 0、没有上升，所以内部 loopback 路径没有打开——ROM 的
`rom_set_loopback_gain`（`0x400067c8`）在这份反汇编里没有任何调用点，因此没有 arity 依据，也就没有声明。

### 校准锁存的轮询：两个可修的点

`wait_rfpll_cal_end` 的轮询走 `rom_i2c_readReg_Mask`，而硬件上 i2c 读路径的 `0x30000` 位不开时每个 cell 都读 0
（`set_rx_gain_testchip_50` 正是先开这几位再读）。`set_rf_freq_offset` 只打开 `0xf00000` 那个字段，所以移植之前
的轮询永远读到 0。现在端口在轮询期间打开读路径、之后恢复原来的门值。

第二点是窗口：`rom_restart_cal` 之后锁定位大约在总第 286 次轮询出现，SDK 的 100 不够。按信道跑一遍（界 3000，
`/tmp/opencode/cal_gate_perchannel.txt`，探针 example 在记录后删除）：那次运行里信道 2、3、4、5、6、8、11、12、14
在总轮询 286 锁上，信道 1、7、9、10、13 在 3256 次内没锁。移植把窗口取 1000（20 ms），于是走查里锁真的被看到：
`examples/wifi_scan.rs` 报信道 1、3、4、6、9 在第 291 次轮询锁定（掩码 `0x012d`）。两次运行锁定的信道集合不同
（`0x012d` 对 `0x2cbe`），所以锁定是逐次变化的，不是固定的每信道属性——能确定的是：没有这两个修正，移植在任何
信道上都报"不锁"。把窗口放宽是移植唯一有意偏离 SDK 的地方。

但即使信道锁上，RX 数据路径仍然不交付描述符：在锁定的信道 14 上驻留 2 秒、每片一次 `check`，仍然
`frames 0`、`rx_ok 0`、`ring owned 24`，FIQ 只有 TX 的 `0x80000`。`Scan` 现在把每次跳跃的 `locked`/`polls` 记成
证据（`Scan::last_tune`、`Scan::locked_channels`），所以判定"是听不到信号还是解调器不工作"的实验仍然是：在板子
旁边开一个 2.4 GHz 热点再跑 `examples/wifi_scan.rs`。

于是 RX 的界线是：软件可修的那部分（`0x60009a34` 使能、门限算术、CCA 字段、`dig_11b_filt_sel`、
`start_dig_rx`、每轮的 `noise_check_loop`、校准读路径的 `0x30000`、窗口从 100 放宽到 1000）已经修好并被硬件
证实；剩下的需要外部条件——部分信道在 3256 次轮询内仍不锁（它读的校准 cell 需要每芯片数据），RX 增益窗口只读，
比较器的信道字段由 ROM 按校准数据编程（移植的 `rx_cmp_field` 对默认 blob 预测 195，硬件读 5），所以判定"是听
不到信号还是解调器不工作"必须有 2.4 GHz 信号源：在板子旁边开一个手机热点再跑 `examples/wifi_scan.rs` 是决定性
的实验。

### 热点跑起来之后：MAC 没有从链上取过任何描述符

热点（2.4 GHz，信道未知）开着的两次运行结果仍然是 0 个网络、`rx_ok 0`、`fiq_errors 0`。按信道的证据：
`/tmp/opencode/scan_hotspot.txt` 里 14 个信道有 5 个在第 291 次轮询锁上（掩码 `0x012d`），但每个信道的驻留
（150 ms，比 beacon 周期长）都没有产生一个描述符。

探针 `/tmp/opencode/canary_probe.txt`（探针 example 在记录后删除）把"解调器不交付"和"链没接好"分开：

```
rx_enable 0x3ff20004 = 0x80000000      rx_head 0x3ff20008 = 0x3fff3a14（= 移植写入的链首）
rx_template 0x3ff2000c = 0x3fff3b4c    buffer_end 0x3ff2007c = 0x3fff3a14
chain_a/b/c 0x3ff20080/84/88 = 0x3fff3a14 / 0x3fff3b4c / 0x3fff3b4c
mask_a 0x3ff20ca8 = 0x10001000         mask_b 0x3ff20cac = 0x55555055（低 28 位是硬件自己的状态）
rx_current 0x3ff20014 = 0x00000000     key_enable 0x3ff2080c = 0x00000000
canaries: buffer start intact 24 / 24, descriptors with bit30 0, ring owned 24
handler entries 8, status_or 0x00800000（只有 bit 23）
```

三点结论：

1. `wDevEnableRx @0x00010cc4` 的 `0x3ff20004 |= 0x80000000` 移植确实执行（`mac::enable_rx()` 在
   `fiq::enable()` 之后，即 `app_main` 的位置），硬件读回 `0x80000000`。所以 RX 使能那一笔不是缺失的写。
2. `0x3ff20014` 是硬件自己维护的"当前描述符"指针（`wDev_ProcessFiq` 里 `puVar21 = _DAT_3ff20014`），它一直是
   0，且每个缓冲区头部的 `0xdeadbeef` canary 都完好——即 `wDev_Initialize` 写入的 canary 没有被 DMA 覆盖过。
   所以 MAC 从来没有取过描述符，也从来没有向 RX 池写过任何一个字节。链、模板、buffer 末尾、掩码这些移植写入
   的值都按写入读回，因此问题不在描述符链的接法，而在解调器没有产生任何帧。
3. 驻留时 ADC 仍然只报噪声底（`0xe98`..`0xdff`，随信道单调下降），`0x60009d68` 的信道字段始终是 `5`，
   FIQ 只有 bit 23。也就是说模拟前端在听，但听不到热点。

这条界线目前不能再往内推：测量的那一半需要槽 4（故障）和槽 59（超时），软件无法驱动它，所以"信号太弱"和
"解调器不工作"两种可能里，被排除的只有移植的接法，前者仍然不能从 ADC 这一侧证伪。本节结论以
`scan_hotspot.txt` 和 `canary_probe.txt` 这两次抓取为准（按惯例探针 example 已删除，未再复跑）。

最后一个可软件验证的环节是 pbus 写是否影响 ADC（`/tmp/opencode/gain_probe.txt`）：`apply_rx_gain` 应用 12 个增益码，
debug mode 下 `pbus_rd(3,1)` 读回 `0x38`——所以 pbus 读在 debug mode 下是可观察的（之前"每个块都报 0"来自
work mode 那次探针）。强制写 `pbus_force_test(3,1,0x60)`、`(3,2,6)` 和块 4/5 的 `0x100` 之后，六个 cell 全部读回写入的值
（`0x60`、`0x6`、`0x100` x4），但 ADC 字段仍是 `0xe98`：强制增益不改变测量。因此增益码能不能落地可以确认，
它驱动的模拟锁存不能——真正决定 RX 灵敏度的仍然是 `set_rx_gain_testchip_50` 的测量那一半，而它需要槽 4
（`rom_dc_iq_est`，故障）和槽 59（`rom_pbus_dco___SA2`，超时）。

顺带修了一个转录错误：`wDev_Initialize` 的尾部 canary 是 `*(undefined4 *)((int)puVar2 + (*puVar1 & 0xfff))`，
长度是作为**整数**加到指针上的，所以它在 `buffer + 1532` **字节**处（缓冲区最后 4 个字节），不是第 1532 个
字。移植原来用 `buffer.add(usable)`，写到缓冲区之外 6128 字节、落在相邻缓冲区里。

### `wDev_Initialize` 里移植没有复现的写

Ghidra 报的两个"offcut"调用 `func_00010b8c(0)` 和 `func_00010b90(1)` 不是未知函数：objdump 里
`.irom0.text + 0xb84..0xb90` 的字面量池是 `0x3ff20800`、`0x30303`、`0x10000000`、`0x3ff20804`，所以这两个调用
就是 `wDev_Crypto_Conf(0/1, ...)`。移植没有执行它——这是 RX 之外的缺失写，一共三处：

- `wDev_Insert_KeyEntry`（Ghidra 的 `func_00010980`/`FUN_00010994`）为 key entry 0 和 1 写入地址
  `ethbroadcast`，并把使能寄存器 `0x3ff2080c` 的 bit 0、bit 1 置起来。硬件读回 `0x3ff2080c = 0`，即移植没有
  编程任何 key entry。
- `wDev_Crypto_Conf` 把 `0x3ff20800`/`0x3ff20804` 写为 `0x30303`（当 `(mode & 0xfb) == 1` 再 OR `0x10000000`），
  而移植只写 `wDev_Initialize` 前半的 `0x30000`（也就是 `wDev_Crypto_Disable` 的那个值）。
- 描述符的 `0xdeadbeef` canary 移植执行了（位置修正后）。

`wDev_Initialize` 的其余写（`0x3ff20178 |= 2`、`0x3ff20400 = 0x76503210`、`0x3ff20404/08 = 0xbbbbbbbb`、
`0x3ff20010 = 0`、`0x3ff2006c |= 0x707`、`0x3ff20c18 = 0x2c9f0300`、`0x3ff20000 &= 0xffffff00 | &= 0xdfffffff`、
三个链寄存器、`0x3ff20008/0c/7c`）移植都执行了，并按上面那次读回逐项对照过。缺失的那三处属于硬件加密引擎，
而 `wDev_ProcessFiq` 对不需要解密的帧（info status bit 27 为 0）直接走 `ppEnqueueRxq`，所以它们不是 RX 交付的
阻塞点。

## 扫描与 RX 策略

`wDev_SetRxPolicy @0x00010520` 只是转发到 `wDev_SetBssid`/`wDev_ClearBssid`，所以扫描使用杂项式（promiscuous）
RX 过滤器：`wdev_go_sniffer @0x00010c8c`（经 `wifi_promiscuous_enable`），它置 sniffer 标志、把模板寄存器指向
链、打开两个加密槽、把两个地址掩码清零且 match 为 `0x10000`、往 FIQ **使能掩码** OR 进 `0xc`、清
`0x60009d44` 的 bit 26、等 15 ms、清 `0x3ff20c94` 的 bit 0。`wdev_exit_sniffer @0x00010df8` 会把其中每一笔写
恢复回去，这正是确认每个写的作用的依据。`POLICY_SCAN = 0x10` 有证据支持（`scan_start` 置 `0x3ff2006c` 的
bit 4，`FUN_0001068c` 清它）。驻留时间按片计算（`SLICE_US = 20_000`）。

## AP 列表与 element 基址

`examples/wifi_scan.rs` 打印的就是 AP 列表：每个 beacon 或 probe response 一行
`ch <n> <bssid> rssi <dBm> dBm <ssid>`，扫描结束时打印条数。列表在这块板子上是空的——不是因为收不到信号，
而是因为 MAC 没有取过任何描述符（见上面"热点跑起来之后"一节），但打印和解析路径本身在硬件上验证过——用 CPU 构造的 beacon 喂进解析路径，行内容正确：

```
A: fc 80 type 0 subtype 8 base 36 ch  6 aabbccddee11 rssi  -60 dBm ssid "probe-ap"
```

这次验证暴露了一个真 bug：`frame::element()` 原来从 body 的第一个字节开始走 tag/len 链，而
`ieee80211_parse_beacon @0x00010724` 的基址是 `pbVar13 = iVar11 + 0x24`，也就是 **frame + 36 = body + 12**
（timestamp 8、beacon interval 2、capability 2），并且 `param_2[5] = iVar8 - pbVar13` 是元素区长度、
`if (*(ushort *)(param_1 + 0x16) < 0xc)` 是"body 短于 12 就返回"的守卫。所以按原来的写法，一个 beacon 的
"元素区"其实是时间戳的 8 个字节，第一个 tag 恰好是 0（SSID），len 是下一个字节，SSID 被读成空——第一次探针
运行打印的就是 `ssid ""`。修正为从 `IE_BASE = 12` 开始走之后，同一帧解析出 `probe-ap`。

反编译还确立了两条语义：SSID 取**第一个** tag 0 的元素（`if (!bVar1) { param_2[9] = pbVar13; bVar1 = true; }`），
元素的步长是 `p += p[1] + 2`（与移植的 `i += 2 + len` 相同）。移植只在 beacon / probe response 上调用
`element()`（`scan.rs::record` 和 `sta.rs` 的 probe-response 分支），所以 `frame + 0x24` 这个基址对移植用到的
每一帧都成立。

探针的原始抓取是 `/tmp/opencode/wifi_rx_selftest3.txt`、`wifi_rx_selftest4.txt`；探针 example 在结论记录后删除。

## 启动顺序

`app_main` 的 `FUN_00010b00 @0x00010b00`：

```
register_chipv6_phy -> phy_disable_agc -> ieee8211_phy_init -> lmacInit -> wDev_Initialize ->
pp_attach -> ieee80211_ifattach -> ets_isr_attach(0, &wDev_ProcessFiq) + ets_isr_unmask(1) ->
phy_enable_agc -> wDevEnableRx
```

`lmacInit @0x000100c8` 设置基础速率（`RC_SetBasicRate(2, 1)`，11 Mbps）、`lmacConfMib[8] = lmacConfMib[9] =
0xb`、turnaround `0x100` 和 `0x200`；ampdu 门限 `0x92a` 未实现。

## 烧写流程

```shell
$ cargo build --release --example wifi_scan
$ esptool --chip esp8266 elf2image -o wifi_scan.bin target/xtensa-esp8266-none-elf/release/examples/wifi_scan
$ esptool -p /dev/ttyUSB0 -c esp8266 write-flash --flash-mode dio --flash-size 4MB --flash-freq 40m \
    0x0 wifi_scan.bin0x00000.bin 0x10000 wifi_scan.bin0x10000.bin
```

必须用 `--flash-mode dio`：这块板子的 QIO 数据读路径返回全 0。构建出错时会静默烧写旧的 ELF，所以在
`elf2image` 之前要确认 `cargo build` 打印了 `Finished`。ELF 没有 `.elf` 后缀。
`esptool v5.6.0 read-flash ADDRESS SIZE OUTPUT` 需要输出文件作为位置参数，且不接受
`--flash-mode`/`--flash-freq`。ROM 横幅和异常打印在 74880 波特，应用输出在 115200；`rst cause:4` 加横幅
表示在 `panic_halt` 下发生了异常。

## 移植状态

TX 在硬件上验证过：MAC 读取移植的描述符、编程 TX 槽、并通过 FIQ 上报。RX 的软件那一半也验证过——每一笔写都按写入读回，包括 `0x60009a34` 的模拟使能、门限和 CCA 字段、`noise_check_loop` 的算术、以及 `init_ring` 的全部 RX 写。数据交付本身没有验证过，而且现在有了否定性的硬件结论：热点在范围内时 MAC 从来没有从链上取过描述符（`0x3ff20014` 保持 0、24 个缓冲区头部 canary 完好、没有描述符带 bit 30、FIQ 状态里没有 bit `0x100`），所以剩下的不是移植的接法，而是解调器的校准那一半——它在这块板子上没有数据来源：AGC 条目（槽 4）故障、槽 59 超时、RX 增益窗口对强制写入的值不响应（pbus 读在块 3、4、5 上是可观察的，只有块 2 报 0）、每芯片的校准扇区已擦除。已移植并在启动流程中运行的是不依赖缺失数据的部分：`gen_rx_gain_table` 计算条目，`apply_rx_gain` 执行围绕它们的寄存器序列，`ram_pbus_set_rxgain` 是反汇编的逐行转录。`set_rx_gain_testchip_50` 的测量那一半没有复现，所以 RX DC 表在这块板子上仍然是空的。

RX 的解析和打印那一半用 CPU 构造的 beacon 在硬件上验证过（见"AP 列表与 element 基址"，那次验证顺带修了
element 基址的 bug）；描述符那一半仍只能等真实的 MAC 完成，因为软件无法把 FIQ 状态置起来。
