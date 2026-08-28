package com.clashedge.android.util

import android.content.Context
import android.util.Log
import com.clashedge.android.model.LogEntry
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlinx.coroutines.flow.MutableStateFlow

/**
 * Minimal logger: keeps an in-memory ring buffer (shown in the Logs screen) and
 * appends to a rotating file in filesDir. The [entries] flow drives the UI.
 */
object Logger {

    const val TAG = "ClashEdge"
    private const val RING_SIZE = 400
    private const val FILE_CAP_MB = 2

    private val _entries = MutableStateFlow<List<LogEntry>>(emptyList())
    val entries: kotlinx.coroutines.flow.StateFlow<List<LogEntry>> = _entries

    private var file: File? = null
    private val fmt = SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS", Locale.US)

    fun init(context: Context) {
        file = File(context.filesDir, "logs").apply { mkdirs() } // dir
            .resolve("clashedge.log")
    }

    fun info(msg: String) = append("INFO", msg)
    fun warn(msg: String) = append("WARN", msg)
    fun error(msg: String) = append("ERROR", msg)

    fun append(level: String, message: String) {
        val entry = LogEntry(System.currentTimeMillis(), level, message)
        _entries.value = (_entries.value + entry).takeLast(RING_SIZE)
        file?.let { safeAppend(it, level, message) }
    }

    private fun safeAppend(file: File, level: String, message: String) {
        try {
            if (file.exists() && file.length() > FILE_CAP_MB * 1024L * 1024L) {
                file.delete()
            }
            val line = "[${fmt.format(Date())}] [$level] $message${System.lineSeparator()}"
            file.appendText(line)
        } catch (t: Throwable) {
            Log.w(TAG, "log write failed", t)
        }
    }
}
