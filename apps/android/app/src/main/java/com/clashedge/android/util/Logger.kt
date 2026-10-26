package com.clashedge.android.util

import android.content.Context
import android.util.Log
import com.clashedge.android.model.LogEntry
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.locks.ReentrantLock
import kotlin.concurrent.withLock
import kotlinx.coroutines.flow.MutableStateFlow

/**
 * Minimal logger: keeps an in-memory ring buffer (shown in the Logs screen) and
 * appends to a rotating file in filesDir. The [entries] flow drives the UI.
 *
 * Thread-safe: uses a lock for all mutations and a thread-local SimpleDateFormat.
 */
object Logger {

    const val TAG = "ClashEdge"
    private const val RING_SIZE = 400
    private const val FILE_CAP_MB = 2

    private val _entries = MutableStateFlow<List<LogEntry>>(emptyList())
    val entries: kotlinx.coroutines.flow.StateFlow<List<LogEntry>> = _entries

    private var file: File? = null
    private val lock = ReentrantLock()

    // Thread-local SimpleDateFormat (not thread-safe)
    private val fmt = ThreadLocal.withInitial {
        SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS", Locale.US)
    }

    fun init(context: Context) {
        lock.withLock {
            file = File(context.filesDir, "logs").apply { mkdirs() }.resolve("clashedge.log")
        }
    }

    fun info(msg: String) = append("INFO", msg)
    fun warn(msg: String) = append("WARN", msg)
    fun error(msg: String) = append("ERROR", msg)

    fun append(level: String, message: String) {
        val entry = LogEntry(System.currentTimeMillis(), level, message)
        lock.withLock {
            _entries.value = (_entries.value + entry).takeLast(RING_SIZE)
            file?.let { safeAppend(it, level, message) }
        }
    }

    private fun safeAppend(file: File, level: String, message: String) {
        try {
            if (file.exists() && file.length() > FILE_CAP_MB * 1024L * 1024L) {
                val oldFile = File(file.parentFile, "${file.name}.old")
                if (oldFile.exists()) oldFile.delete()
                file.renameTo(oldFile)
            }
            val line = "[${fmt.get().format(Date())}] [$level] $message${System.lineSeparator()}"
            file.appendText(line)
        } catch (t: Throwable) {
            android.util.Log.w(TAG, "log write failed", t)
        }
    }
}
