//! 诊断：打印全部蓝牙 PnP 节点的原始电量属性（排查电量挂在哪个节点）。

fn main() {
    for (inst, name, level, charging, rank, le_root) in wick_lib::providers::bluetooth::debug_bth_nodes() {
        println!(
            "r{rank}{} | lvl={:>3} chg={:?} | {name:20} | {inst}",
            if le_root { "/LE" } else { "" },
            level.map(|l| l.to_string()).unwrap_or("--".into()),
            charging
        );
    }
}
