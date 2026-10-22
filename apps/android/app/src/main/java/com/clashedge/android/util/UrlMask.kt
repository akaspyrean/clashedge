package com.clashedge.android.util

import android.net.Uri

/**
 * 订阅地址显示脱敏（与 Windows 端 maskUrl 同策略，前后端统一）：
 *  保留 scheme://host[:port]、路径结构与 query 键名；所有疑似 token 的路径段与
 *  query 值一律换成 ••••••••。解析失败返回 "…"（不透出原文）。
 */
fun maskUrl(url: String): String {
    return try {
        val u = Uri.parse(url)
        // 遮蔽占位（固定长度，不泄露原值长度）
        val MASK = "••••••••"

        val path = if (u.path != null && u.path != "/") {
            val segs = u.path!!.replace("^/+".toRegex(), "").split("/")
            // 遮蔽所有形似 token 的路径段（≥8 字符、仅字母数字下划线连字符且含数字）
            val masked = segs.map { seg ->
                if (isTokenLikeSegment(seg)) "••••••••" else seg
            }
            "/${segs.joinToString("/")}"
        } else ""

        val query = if (u.query != null) {
            "?${u.getQueryParameterNames().joinToString("&") { "$it=••••••••" }}"
        } else ""

        "${u.scheme}://${u.host}${if (u.port != -1) ":${u.port}" else ""}$path$query"
    } catch (e: Exception) {
        "…"
    }
}

/** 形似 token 的路径段：≥8 字符、仅 `[A-Za-z0-9_-]` 且含数字。 */
private fun isTokenLikeSegment(seg: String): Boolean {
    return seg.length >= 8 && "^[A-Za-z0-9_-]+\$".toRegex().matches(seg) && seg.contains(Regex("\\d"))
}