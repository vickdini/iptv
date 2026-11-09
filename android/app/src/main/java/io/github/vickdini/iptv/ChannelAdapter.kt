package io.github.vickdini.iptv

import android.graphics.Color
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.TextView
import androidx.recyclerview.widget.RecyclerView

/**
 * Rows of the channel list: number, name, ▶ for the channel playing, ★ for
 * favorites. Works from an index array plus names fetched once, so updating
 * the list (channels found dead while checking) stays cheap.
 */
class ChannelAdapter(
    private val onClick: (Int) -> Unit,
    private val onLongClick: (Int) -> Unit,
) : RecyclerView.Adapter<ChannelAdapter.Holder>() {

    /** All channel names by index (set once after the playlist loads). */
    var names: Array<String> = emptyArray()

    /** Channel indices shown, ascending. */
    var items: IntArray = IntArray(0)
        private set
    private var favorites: Set<Int> = emptySet()
    private var current = -1

    fun submit(items: IntArray, favorites: IntArray, current: Int) {
        this.items = items
        this.favorites = favorites.toHashSet()
        this.current = current
        notifyDataSetChanged()
    }

    fun setCurrent(idx: Int) {
        val old = current
        current = idx
        for (i in intArrayOf(old, idx)) {
            val pos = positionOf(i)
            if (items.getOrNull(pos) == i) notifyItemChanged(pos)
        }
    }

    /** Position of channel [idx], or of the nearest following one (items are sorted). */
    fun positionOf(idx: Int): Int {
        var lo = 0
        var hi = items.size
        while (lo < hi) {
            val mid = (lo + hi) ushr 1
            if (items[mid] < idx) lo = mid + 1 else hi = mid
        }
        return lo.coerceAtMost((items.size - 1).coerceAtLeast(0))
    }

    override fun getItemCount() = items.size

    override fun getItemId(position: Int) = items[position].toLong()

    override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): Holder =
        Holder(LayoutInflater.from(parent.context).inflate(R.layout.item_channel, parent, false))

    override fun onBindViewHolder(holder: Holder, position: Int) {
        val idx = items[position]
        val playing = idx == current
        holder.number.text = (idx + 1).toString()
        holder.name.text = buildString {
            if (playing) append("▶ ")
            append(names.getOrElse(idx) { "" })
            if (idx in favorites) append("  ★")
        }
        holder.name.setTextColor(if (playing) Color.rgb(255, 208, 64) else Color.WHITE)
        holder.itemView.setOnClickListener { onClick(idx) }
        holder.itemView.setOnLongClickListener { onLongClick(idx); true }
    }

    class Holder(view: View) : RecyclerView.ViewHolder(view) {
        val number: TextView = view.findViewById(R.id.number)
        val name: TextView = view.findViewById(R.id.name)
    }
}
