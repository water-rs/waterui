package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Create
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Email
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Send
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.outlined.Email
import androidx.compose.material.icons.outlined.Notifications
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material3.Badge
import androidx.compose.material3.Button
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

private data class RMessage(
    val sender: String,
    val time: String,
    val recipients: String,
    val body: String,
    val signature: String?,
    val avatarIndex: Int,
)

private data class RThread(
    val sender: String,
    val time: String,
    val subject: String,
    val snippet: String,
    val avatarIndex: Int,
    val hasPhoto: Boolean,
    val messages: List<RMessage>,
)

private val THREADS = listOf(
    RThread(
        sender = "老强", time = "10 min ago", subject = "豆花鱼",
        snippet = "最近忙吗？昨晚我去了你最爱的那家饭馆，点了他们的特色豆花鱼，吃着吃着就想你了。",
        avatarIndex = 8, hasPhoto = false,
        messages = listOf(
            RMessage(
                "老强", "10 min ago", "To me",
                "最近忙吗？昨晚我去了你最爱的那家饭馆，点了他们的特色豆花鱼，吃着吃着就想你了。有空过来，我请你吃。",
                "老强", 8,
            ),
        ),
    ),
    RThread(
        sender = "So Duri", time = "20 min ago", subject = "Dinner Club",
        snippet = "I think it's time for us to finally try that new noodle shop downtown that doesn't use me…",
        avatarIndex = 3, hasPhoto = false,
        messages = listOf(
            RMessage(
                "So Duri", "20 min ago", "To me, Ziad, and Lily",
                "I think it's time for us to finally try that new noodle shop downtown that doesn't use menus. Anyone else have other suggestions for dinner club this week? I'm so intrigued by this idea of a noodle restaurant where no one gets to order for themselves – could be fun, or terrible, or both :)",
                "So", 3,
            ),
            RMessage(
                "Me", "4 min ago", "To me, Ziad, and Lily",
                "Yes! I forgot about that place! I'm definitely up for taking a risk this week and handing control over to someone else. Let's do it.",
                null, 10,
            ),
            RMessage(
                "Lily MacDonald", "1 hour ago", "To me, Ziad, and So",
                "Count me in! I've been wanting to try that place since it opened. Thursday works best for me.",
                "Lily", 1,
            ),
        ),
    ),
    RThread(
        sender = "Lily MacDonald", time = "2 hours ago",
        subject = "This food show is made for you",
        snippet = "Ping– you'd love this new food show I started watching. It's produced by a Thai drummer…",
        avatarIndex = 1, hasPhoto = true,
        messages = listOf(
            RMessage(
                "Lily MacDonald", "2 hours ago", "To me and Karthik",
                "Ping– you'd love this new food show I started watching. It's produced by a Thai drummer who started a noodle cart during lockdowns, and every episode ends with a cook-along. Attached a still from last night's episode.",
                "Lily", 1,
            ),
        ),
    ),
)

private val AVATAR_COLORS = listOf(
    Color(0xFFEF9A9A), Color(0xFFCE93D8), Color(0xFF90CAF9),
    Color(0xFF80CBC4), Color(0xFFA5D6A7), Color(0xFFFFF59D),
    Color(0xFFFFCC80), Color(0xFFF48FB1), Color(0xFFB39DDB),
    Color(0xFF9FA8DA), Color(0xFF8D6E63),
)

/**
 * Compose twin of the reply dogfood app (the Compose-samples Reply client):
 * navigation rail (Menu + Compose FAB + Mail badge-4/Notes/Chat/Meet),
 * search bar, the three seeded threads, and the reading pane with avatars,
 * Reply/Reply-all pills and the Delete/More icon buttons. Avatars are drawn
 * initial circles — the sample's network avatar URLs are pinned to local
 * renderings per the no-network rule.
 */
@Composable
fun ReplyTwin() {
    var rail by remember { mutableIntStateOf(0) }
    var selected by remember { mutableIntStateOf(0) }
    var starred by remember { mutableStateOf(setOf<Int>()) }

    Row(Modifier.fillMaxSize()) {
        NavigationRail(modifier = Modifier.testTag("reply:rail")) {
            IconButton(
                onClick = { BenchMarkers.tap() },
                modifier = Modifier.testTag("rail:menu"),
            ) { Icon(Icons.Filled.Menu, "Menu") }
            ExtendedFloatingActionButton(
                onClick = { BenchMarkers.tap() },
                icon = { Icon(Icons.Filled.Create, "Compose") },
                text = { Text("Compose") },
                modifier = Modifier.testTag("rail:compose"),
            )
            Spacer(Modifier.height(16.dp))
            NavigationRailItem(
                selected = rail == 0,
                onClick = { BenchMarkers.tap(); rail = 0 },
                icon = {
                    Box {
                        Icon(Icons.Outlined.Email, "Mail")
                        Badge(Modifier.align(Alignment.TopEnd)) { Text("4") }
                    }
                },
                label = { Text("Mail") },
                modifier = Modifier.testTag("rail:mail"),
            )
            NavigationRailItem(
                selected = rail == 1,
                onClick = { BenchMarkers.tap(); rail = 1 },
                icon = { Icon(Icons.Filled.Create, "Notes") },
                label = { Text("Notes") },
                modifier = Modifier.testTag("rail:notes"),
            )
            NavigationRailItem(
                selected = rail == 2,
                onClick = { BenchMarkers.tap(); rail = 2 },
                icon = { Icon(Icons.Filled.Send, "Chat") },
                label = { Text("Chat") },
                modifier = Modifier.testTag("rail:chat"),
            )
            NavigationRailItem(
                selected = rail == 3,
                onClick = { BenchMarkers.tap(); rail = 3 },
                icon = { Icon(Icons.Outlined.PlayArrow, "Meet") },
                label = { Text("Meet") },
                modifier = Modifier.testTag("rail:meet"),
            )
        }

        Column(Modifier.fillMaxSize()) {
            // Search bar.
            OutlinedTextField(
                value = "",
                onValueChange = {},
                modifier = Modifier.fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp)
                    .testTag("reply:search"),
                singleLine = true,
                placeholder = { Text("Search replies") },
                leadingIcon = { Icon(Icons.Filled.Menu, null) },
            )

            Row(Modifier.fillMaxSize()) {
                // Thread list.
                Column(
                    Modifier.weight(1f).fillMaxHeight()
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 12.dp)
                        .testTag("reply:threads"),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    THREADS.forEachIndexed { i, thread ->
                        ThreadCard(
                            thread = thread,
                            selected = selected == i,
                            starred = i in starred,
                            onSelect = {
                                BenchMarkers.tap()
                                selected = i
                            },
                            onStar = {
                                BenchMarkers.tap()
                                starred = if (i in starred) starred - i else starred + i
                            },
                        )
                    }
                }

                // Reading pane.
                ThreadDetail(
                    thread = THREADS[selected],
                    modifier = Modifier.weight(1f).fillMaxHeight()
                        .testTag("reply:detail"),
                )
            }
        }
    }
}

@Composable
private fun Avatar(name: String, index: Int) {
    Box(
        Modifier.size(40.dp).clip(CircleShape)
            .background(AVATAR_COLORS[index % AVATAR_COLORS.size]),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            name.take(2),
            style = MaterialTheme.typography.labelMedium,
            color = Color.White,
        )
    }
}

@Composable
private fun ThreadCard(
    thread: RThread,
    selected: Boolean,
    starred: Boolean,
    onSelect: () -> Unit,
    onStar: () -> Unit,
) {
    Surface(
        onClick = onSelect,
        modifier = Modifier.fillMaxWidth().testTag("reply:thread-${thread.subject}"),
        shape = RoundedCornerShape(16.dp),
        color = if (selected) {
            MaterialTheme.colorScheme.secondaryContainer
        } else {
            MaterialTheme.colorScheme.surfaceContainerHigh
        },
    ) {
        Column(Modifier.fillMaxWidth().padding(20.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Avatar(thread.sender, thread.avatarIndex)
                Column(Modifier.weight(1f)) {
                    Text(thread.sender, style = MaterialTheme.typography.labelMedium)
                    Text(
                        thread.time,
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.outline,
                    )
                }
                IconButton(onClick = onStar, modifier = Modifier.size(28.dp).testTag("reply:star")) {
                    Icon(
                        Icons.Filled.Star,
                        "star",
                        tint = if (starred) {
                            MaterialTheme.colorScheme.tertiary
                        } else {
                            MaterialTheme.colorScheme.outline
                        },
                    )
                }
            }
            Text(thread.subject, style = MaterialTheme.typography.bodyMedium)
            Text(
                thread.snippet,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
            )
            if (thread.hasPhoto) {
                Box(
                    Modifier.fillMaxWidth().height(160.dp)
                        .clip(RoundedCornerShape(16.dp))
                        .background(MaterialTheme.colorScheme.surfaceVariant),
                    contentAlignment = Alignment.Center,
                ) { CaptionText("attached still") }
            }
        }
    }
}

@Composable
private fun ThreadDetail(thread: RThread, modifier: Modifier = Modifier) {
    Column(modifier.verticalScroll(rememberScrollState()).padding(16.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(thread.subject, style = MaterialTheme.typography.titleLarge)
            Spacer(Modifier.weight(1f))
            IconButton(onClick = { BenchMarkers.tap() }, modifier = Modifier.testTag("reply:delete")) {
                Icon(Icons.Filled.Delete, "Delete")
            }
            IconButton(onClick = { BenchMarkers.tap() }, modifier = Modifier.testTag("reply:more")) {
                Icon(Icons.Filled.MoreVert, "More")
            }
        }
        Text(
            "${thread.messages.size} Messages",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.outline,
        )
        Spacer(Modifier.height(12.dp))
        for (message in thread.messages) {
            Surface(
                modifier = Modifier.fillMaxWidth().padding(vertical = 6.dp),
                shape = RoundedCornerShape(16.dp),
                color = MaterialTheme.colorScheme.surfaceContainer,
            ) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Avatar(message.sender, message.avatarIndex)
                        Spacer(Modifier.width(10.dp))
                        Column {
                            Text(message.sender, style = MaterialTheme.typography.labelMedium)
                            Text(
                                message.recipients,
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.outline,
                            )
                        }
                    }
                    Text(message.body, style = MaterialTheme.typography.bodyMedium)
                    message.signature?.let {
                        Text(it, style = MaterialTheme.typography.bodyMedium)
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        OutlinedButton(onClick = { BenchMarkers.tap() }) { Text("Reply") }
                        OutlinedButton(onClick = { BenchMarkers.tap() }) { Text("Reply all") }
                    }
                }
            }
        }
    }
}
