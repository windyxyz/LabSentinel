// 主题与配色：常量与 web/src/style.css 的 CSS 变量一一对应。
// 后续页面一律引用 AppColors，禁止散落写死色值，保证与 Web 端观感一致。
import 'package:flutter/material.dart';

/// Web 端配色（web/src/style.css 的 :root）
class AppColors {
  AppColors._(); // 纯静态常量类，禁止实例化

  static const Color bg = Color(0xFF0F1419); // --bg      页面底色
  static const Color panel = Color(0xFF1A2129); // --panel   卡片/面板
  static const Color border = Color(0xFF2A3441); // --border  描边
  static const Color text = Color(0xFFD8E1EA); // --text    正文
  static const Color muted = Color(0xFF7A8899); // --muted   弱化文字
  static const Color accent = Color(0xFF4DA3FF); // --accent  主题强调色
  static const Color l1 = Color(0xFFF5C518); // --l1      L1 提示
  static const Color l2 = Color(0xFFFF8C42); // --l2      L2 警告
  static const Color l3 = Color(0xFFFF4D4D); // --l3      L3 严重

  /// 事件级别(1/2/3) → 级别色；未知级别回退为弱化色
  static Color levelColor(int level) => switch (level) {
        1 => l1,
        2 => l2,
        3 => l3,
        _ => muted,
      };
}

/// 全局深色主题：以 --accent 为种子生成 Material 3 深色配色，
/// 再覆写背景/面板/描边等关键色，与 Web 端对齐。
ThemeData buildAppTheme() {
  final scheme = ColorScheme.fromSeed(
    seedColor: AppColors.accent, // seedColor #4da3ff
    brightness: Brightness.dark,
  ).copyWith(
    primary: AppColors.accent,
    surface: AppColors.panel, // 卡片/导航底色 #1a2129
  );

  const borderShape = OutlineInputBorder(
    borderRadius: BorderRadius.all(Radius.circular(6)),
    borderSide: BorderSide(color: AppColors.border),
  );

  return ThemeData(
    useMaterial3: true,
    colorScheme: scheme,
    scaffoldBackgroundColor: AppColors.bg, // #0f1419
    dividerColor: AppColors.border,
    appBarTheme: const AppBarTheme(
      backgroundColor: AppColors.bg,
      foregroundColor: AppColors.text,
      elevation: 0,
      centerTitle: false,
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: AppColors.bg, // 对齐 Web 端 input 背景
      hintStyle: const TextStyle(color: AppColors.muted),
      border: borderShape,
      enabledBorder: borderShape,
    ),
    snackBarTheme: const SnackBarThemeData(
      backgroundColor: AppColors.panel,
      contentTextStyle: TextStyle(color: AppColors.text),
      behavior: SnackBarBehavior.floating,
    ),
  );
}
