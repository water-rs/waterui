package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items as gridItems
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Email
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.PhotoLibrary
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Star
import androidx.compose.material3.Badge
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

private data class Message(
    val id: Int,
    val sender: String,
    val subject: String,
    val preview: String,
    val unread: Boolean,
    val flagged: Boolean,
)

private fun seedMessages() = mutableListOf(
    Message(0, "Ada Lovelace", "Analytical engine notes", "The engine weaves algebraic patterns.", true, false),
    Message(1, "Grace Hopper", "Compiler timings", "Shaved another pass off the linker.", false, false),
    Message(2, "Alan Kay", "On messaging", "The big idea is messaging, not objects.", true, false),
    Message(3, "Barbara Liskov", "Substitution review", "Subtypes must not surprise their callers.", false, false),
    Message(4, "Ken Thompson", "Pipes", "One tool, one job, composed by the shell.", true, false),
    Message(5, "Margaret Hamilton", "Priority displays", "Overload handling saved the landing.", false, false),
)

private enum class Pane { Inbox, Library, Gallery, Settings }
private sealed class MailRoute { data object Inbox : MailRoute(); data class Detail(val id: Int) : MailRoute(); data object Compose : MailRoute() }
private sealed class SettingsRoute { data object Root : SettingsRoute(); data object Appearance : SettingsRoute(); data object About : SettingsRoute() }

private val PHOTO_COLORS = listOf(
    Color(0xFFEF4444), Color(0xFFF59E0B), Color(0xFF10B981),
    Color(0xFF3B82F6), Color(0xFF8B5CF6), Color(0xFFEC4899),
)

/**
 * Compose twin of the navigation fixture: four bottom tabs (Inbox with the
 * unread badge, Library, Gallery, Settings) each carrying its own back stack
 * — matching the fixture's per-tab NavigationStack behavior.
 */
@Composable
fun NavigationTwin() {
    var pane by remember { mutableStateOf(Pane.Inbox) }
    val messages = remember { seedMessages() }
    var refresh by remember { mutableIntStateOf(0) }
    val mailStack = remember { mutableListOf<MailRoute>(MailRoute.Inbox) }
    val settingsStack = remember { mutableListOf<SettingsRoute>(SettingsRoute.Root) }
    var gallerySelection by remember { mutableStateOf(-1) }
    var album by remember { mutableStateOf("Recents") }

    fun bump() { refresh += 1 }

    Scaffold(
        bottomBar = {
            NavigationBar {
                NavigationBarItem(
                    selected = pane == Pane.Inbox,
                    onClick = { BenchMarkers.tap(); pane = Pane.Inbox },
                    icon = {
                        val unread = messages.count { it.unread } + refresh * 0
                        if (unread > 0) {
                            Box {
                                Icon(Icons.Filled.Email, contentDescription = "Inbox")
                                Badge(Modifier.align(Alignment.TopEnd)) { Text("$unread") }
                            }
                        } else {
                            Icon(Icons.Filled.Email, contentDescription = "Inbox")
                        }
                    },
                    label = { Text("Inbox") },
                    modifier = Modifier.testTag("tab:inbox"),
                )
                NavigationBarItem(
                    selected = pane == Pane.Library,
                    onClick = { BenchMarkers.tap(); pane = Pane.Library },
                    icon = { Icon(Icons.Filled.PhotoLibrary, contentDescription = "Library") },
                    label = { Text("Library") },
                    modifier = Modifier.testTag("tab:library"),
                )
                NavigationBarItem(
                    selected = pane == Pane.Gallery,
                    onClick = { BenchMarkers.tap(); pane = Pane.Gallery },
                    icon = { Icon(Icons.Filled.Star, contentDescription = "Gallery") },
                    label = { Text("Gallery") },
                    modifier = Modifier.testTag("tab:gallery"),
                )
                NavigationBarItem(
                    selected = pane == Pane.Settings,
                    onClick = { BenchMarkers.tap(); pane = Pane.Settings },
                    icon = { Icon(Icons.Filled.Settings, contentDescription = "Settings") },
                    label = { Text("Settings") },
                    modifier = Modifier.testTag("tab:settings"),
                )
            }
        },
    ) { padding ->
        Box(Modifier.padding(padding)) {
            when (pane) {
                Pane.Inbox -> InboxPane(messages, mailStack, ::bump)
                Pane.Library -> LibraryPane(album) { album = it }
                Pane.Gallery -> GalleryPane(gallerySelection) { gallerySelection = it }
                Pane.Settings -> SettingsPane(settingsStack)
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun InboxPane(
    messages: MutableList<Message>,
    stack: MutableList<MailRoute>,
    bump: () -> Unit,
) {
    var query by remember { mutableStateOf("") }
    var editing by remember { mutableStateOf(false) }
    var draftSubject by remember { mutableStateOf("") }
    var draftBody by remember { mutableStateOf("") }

    when (val route = stack.last()) {
        MailRoute.Inbox -> Column(Modifier.fillMaxSize()) {
            TopAppBar(
                title = { Text("Inbox") },
                actions = {
                    TextButton(onClick = {
                        BenchMarkers.tap()
                        editing = !editing
                    }, modifier = Modifier.testTag("nav:edit")) {
                        Text(if (editing) "Done" else "Edit")
                    }
                    TextButton(onClick = {
                        BenchMarkers.tap()
                        messages.replaceAll { it.copy(unread = false) }
                        bump()
                    }, modifier = Modifier.testTag("nav:mark-all-read")) {
                        Text("Mark All Read")
                    }
                    TextButton(onClick = {
                        BenchMarkers.tap()
                        stack += MailRoute.Compose
                    }, modifier = Modifier.testTag("nav:compose")) {
                        Text("Compose")
                    }
                },
            )
            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp)
                    .testTag("nav:search"),
                singleLine = true,
                placeholder = { Text("Search") },
            )
            val unread = messages.count { it.unread }
            CaptionText(
                "$unread unread",
                modifier = Modifier.padding(12.dp),
            )
            LazyColumn(Modifier.fillMaxSize().testTag("nav:message-list")) {
                items(
                    messages.filter {
                        query.isEmpty() ||
                            it.subject.contains(query, true) ||
                            it.sender.contains(query, true)
                    },
                    key = { it.id },
                ) { message ->
                    Column(
                        Modifier.fillMaxWidth()
                            .clickable {
                                BenchMarkers.tap()
                                // Opening marks read — the fixture's
                                // destination lifecycle event.
                                messages[message.id] = message.copy(unread = false)
                                bump()
                                stack += MailRoute.Detail(message.id)
                            }
                            .padding(horizontal = 16.dp, vertical = 10.dp)
                            .testTag("nav:message-${message.id}"),
                    ) {
                        Row(
                            Modifier.fillMaxWidth(),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                message.sender,
                                style = MaterialTheme.typography.titleSmall,
                                fontWeight = if (message.unread) FontWeight.Bold else FontWeight.Normal,
                            )
                            Spacer(Modifier.weight(1f))
                            if (message.flagged) {
                                Icon(
                                    Icons.Filled.Star, "flagged",
                                    tint = MaterialTheme.colorScheme.tertiary,
                                    modifier = Modifier.size(16.dp),
                                )
                            }
                        }
                        Text(message.subject, style = MaterialTheme.typography.bodyMedium)
                        Text(
                            message.preview,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    HorizontalDivider()
                }
            }
        }
        is MailRoute.Detail -> {
            val message = messages.first { it.id == route.id }
            Column(Modifier.fillMaxSize().padding(16.dp)) {
                TopAppBar(
                    title = { Text(message.subject) },
                    navigationIcon = {
                        TextButton(onClick = {
                            BenchMarkers.tap()
                            stack.removeLast()
                        }, modifier = Modifier.testTag("nav:back")) { Text("Inbox") }
                    },
                    actions = {
                        TextButton(onClick = {
                            BenchMarkers.tap()
                            messages[message.id] = message.copy(flagged = !message.flagged)
                            bump()
                        }, modifier = Modifier.testTag("nav:flag")) {
                            Text(if (message.flagged) "Unflag" else "Flag")
                        }
                    },
                )
                SubheadlineText(message.sender)
                Spacer(Modifier.height(12.dp))
                BodyText(message.preview)
                BodyText(
                    "The rest of the thread continues here; the fixture's detail "
                        + "body is the message preview plus this filler paragraph.",
                )
            }
        }
        MailRoute.Compose -> Column(Modifier.fillMaxSize().padding(16.dp)) {
            TopAppBar(title = { Text("Compose") })
            OutlinedTextField(
                value = draftSubject,
                onValueChange = { draftSubject = it },
                modifier = Modifier.fillMaxWidth().testTag("nav:draft-subject"),
                label = { Text("Subject") },
            )
            OutlinedTextField(
                value = draftBody,
                onValueChange = { draftBody = it },
                modifier = Modifier.fillMaxWidth().testTag("nav:draft-body"),
                label = { Text("Body") },
            )
            BodyText(
                "This compose page stays inside the Inbox stack: dismiss it "
                    + "with the back arrow, the Cancel button or the platform "
                    + "back gesture, then use Cancel.",
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = {
                    BenchMarkers.tap()
                    if (draftSubject.isNotEmpty() || draftBody.isNotEmpty()) {
                        messages += Message(
                            messages.size, "me", draftSubject,
                            draftBody.take(80), unread = false, flagged = false,
                        )
                    }
                    draftSubject = ""; draftBody = ""
                    bump()
                    stack.removeLast()
                }, modifier = Modifier.testTag("nav:send")) { Text("Send") }
                Button(onClick = {
                    BenchMarkers.tap()
                    stack.removeLast()
                }, modifier = Modifier.testTag("nav:cancel")) { Text("Cancel") }
            }
        }
    }
}

@Composable
private fun LibraryPane(selected: String, onSelect: (String) -> Unit) {
    val albums = listOf("Recents" to 128, "Favorites" to 12, "Shared with You" to 41)
    Row(Modifier.fillMaxSize()) {
        Column(Modifier.width(160.dp)) {
            for ((title, count) in albums) {
                Column(
                    Modifier.fillMaxWidth()
                        .clickable {
                            BenchMarkers.tap()
                            onSelect(title)
                        }
                        .background(
                            if (selected == title) {
                                MaterialTheme.colorScheme.secondaryContainer
                            } else {
                                Color.Transparent
                            },
                        )
                        .padding(12.dp)
                        .testTag("nav:album-$title"),
                ) {
                    BodyText(title)
                    CaptionText("$count")
                }
            }
        }
        Column(Modifier.weight(1f).padding(12.dp)) {
            val count = albums.first { it.first == selected }.second
            HeadlineText(selected)
            BodyText("$count photos")
            LazyVerticalGrid(
                columns = GridCells.Fixed(3),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                gridItems((0 until minOf(count, 24)).toList()) { i ->
                    Box(
                        Modifier.size(96.dp)
                            .clip(RoundedCornerShape(4.dp))
                            .background(PHOTO_COLORS[i % PHOTO_COLORS.size]),
                    )
                }
            }
        }
    }
}

@Composable
private fun GalleryPane(selection: Int, onSelect: (Int) -> Unit) {
    if (selection < 0) {
        LazyVerticalGrid(
            columns = GridCells.Fixed(3),
            modifier = Modifier.fillMaxSize().testTag("nav:gallery-grid"),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            gridItems(PHOTO_COLORS.indices.toList()) { i ->
                Box(
                    Modifier.size(160.dp, 120.dp)
                        .clickable {
                            BenchMarkers.tap()
                            onSelect(i)
                        }
                        .background(PHOTO_COLORS[i])
                        .testTag("nav:photo-$i"),
                )
            }
        }
    } else {
        Column(Modifier.fillMaxSize().padding(16.dp)) {
            TextButton(onClick = {
                BenchMarkers.tap()
                onSelect(-1)
            }, modifier = Modifier.testTag("nav:gallery-back")) { Text("Gallery") }
            Box(
                Modifier.fillMaxWidth().height(280.dp)
                    .clip(RoundedCornerShape(12.dp))
                    .background(PHOTO_COLORS[selection]),
            )
            BodyText(
                "A matched zoom transition ties tile $selection to this page; "
                    + "the gallery and detail share the photo's identity.",
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SettingsPane(stack: MutableList<SettingsRoute>) {
    when (stack.last()) {
        SettingsRoute.Root -> Column(Modifier.fillMaxSize()) {
            TopAppBar(title = { Text("Settings") })
            Column(Modifier.padding(16.dp)) {
                Surface(
                    onClick = {
                        BenchMarkers.tap()
                        stack += SettingsRoute.Appearance
                    },
                    modifier = Modifier.fillMaxWidth().testTag("nav:appearance"),
                ) {
                    BodyText("Appearance")
                }
                Surface(
                    onClick = {
                        BenchMarkers.tap()
                        stack += SettingsRoute.About
                    },
                    modifier = Modifier.fillMaxWidth().testTag("nav:about"),
                ) {
                    BodyText("About")
                }
            }
        }
        SettingsRoute.Appearance -> Column(Modifier.fillMaxSize().padding(16.dp)) {
            TextButton(onClick = {
                BenchMarkers.tap()
                stack.removeLast()
            }, modifier = Modifier.testTag("nav:settings-back")) { Text("Settings") }
            HeadlineText("Appearance")
            BodyText("Theme follows the system; this page is the pushed sub-page.")
        }
        SettingsRoute.About -> Column(Modifier.fillMaxSize().padding(16.dp)) {
            TextButton(onClick = {
                BenchMarkers.tap()
                stack.removeLast()
            }, modifier = Modifier.testTag("nav:settings-back")) { Text("Settings") }
            HeadlineText("About")
            BodyText("WaterUI navigation reference — Kotlin/Compose counterpart.")
        }
    }
}
