//! Classify target failures by the operation that failed; independent of UI text from the OS.
use std::io::ErrorKind;

#[derive(Clone, Copy)]
pub enum Operation {
    Directory,
    Write,
    Publish,
}

pub fn classify(operation: Operation, kind: ErrorKind) -> (&'static str, &'static str) {
    use ErrorKind::*;
    use Operation::*;
    match (operation, kind) {
        (_, StorageFull) => (
            "EXPORT_NO_SPACE",
            "目标磁盘空间不足，请释放空间后重新预演导出",
        ),
        (_, PermissionDenied) => (
            "PERMISSION_DENIED",
            "无法访问导出目标，请检查目录权限，并通过系统选择器重新选择目标后预演",
        ),
        (_, NotADirectory) | (Directory, AlreadyExists | InvalidInput | NotFound) => (
            "EXPORT_TARGET_UNSAFE",
            "目标路径不是可安全访问的文件夹，可能被文件或链接占用；请重新选择目标目录并预演",
        ),
        (Publish, InvalidInput | InvalidData) => (
            "EXPORT_TARGET_UNSAFE",
            "目标目录或临时副本在执行时发生变化，请重新选择目标并预演；已完成副本保留",
        ),
        (_, AlreadyExists) => (
            "EXPORT_CONFLICT",
            "输出路径已被占用，请重新预演并选择自动改名或跳过；已有文件不会覆盖",
        ),
        (Publish, _) => (
            "EXPORT_CONFLICT",
            "目标无法安全发布，请重新选择目标并预演；已完成副本保留",
        ),
        (Directory, _) => (
            "INTERNAL",
            "无法创建或访问目标子目录，请检查目标位置和权限后重新预演",
        ),
        (Write, _) => (
            "INTERNAL",
            "无法写入导出目标，请重新选择目录并预演；已完成副本保留",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occupied_parent_is_an_unsafe_target_but_occupied_output_is_a_conflict() {
        assert_eq!(
            classify(Operation::Directory, ErrorKind::AlreadyExists).0,
            "EXPORT_TARGET_UNSAFE"
        );
        for op in [Operation::Write, Operation::Publish] {
            assert_eq!(classify(op, ErrorKind::AlreadyExists).0, "EXPORT_CONFLICT");
        }
    }

    #[test]
    fn disk_full_and_permission_denial_keep_their_distinct_actions() {
        for op in [Operation::Directory, Operation::Write, Operation::Publish] {
            assert_eq!(classify(op, ErrorKind::StorageFull).0, "EXPORT_NO_SPACE");
            let permission = classify(op, ErrorKind::PermissionDenied);
            assert_eq!(permission.0, "PERMISSION_DENIED");
            assert!(permission.1.contains("系统选择器"));
        }
    }

    #[test]
    fn changed_or_unusable_target_instructs_reselection_without_overwrite() {
        for kind in [ErrorKind::InvalidInput, ErrorKind::InvalidData] {
            let failure = classify(Operation::Publish, kind);
            assert_eq!(failure.0, "EXPORT_TARGET_UNSAFE");
            assert!(failure.1.contains("重新选择"));
            assert!(failure.1.contains("已完成副本保留"));
        }
        for op in [Operation::Directory, Operation::Write, Operation::Publish] {
            assert_eq!(
                classify(op, ErrorKind::NotADirectory).0,
                "EXPORT_TARGET_UNSAFE"
            );
        }
    }

    #[test]
    fn unknown_failures_offer_an_action_instead_of_referring_to_the_same_details() {
        for op in [Operation::Directory, Operation::Write] {
            let failure = classify(op, ErrorKind::Other);
            assert_eq!(failure.0, "INTERNAL");
            assert!(failure.1.contains("重新"));
            assert!(!failure.1.contains("查看失败明细"));
        }
    }
}
