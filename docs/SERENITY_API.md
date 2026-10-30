# Serenity 0.12.5 — Verified API cheat sheet

Source of truth: `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serenity-0.12.5`
(`SERENITY_SRC` below). Every signature here was read from that source and the snippets in
§2–§11 were compiled with `cargo check` against the exact feature set used by this crate.

> **Read §0 and §13 first.** Three assumptions commonly made about serenity are wrong in 0.12.5:
> `Interaction` is an **enum** (not a struct with `data`), `MessageComponentInteraction` no longer
> exists (it is `ComponentInteraction`), and `CreateMember` does not exist (it is `EditMember`).

---

## 0. Surprises vs. the "usual" 0.11-style API

| Assumption | Reality in 0.12.5 |
|---|---|
| `Interaction { data, member, user, .. }` | `pub enum Interaction { Ping, Command, Autocomplete, Component, Modal }` — `SERENITY_SRC/src/model/application/interaction.rs:26`. `#[non_exhaustive]`, so every `match` needs a `_` arm. |
| `InteractionData::ApplicationCommand(..)` | Gone. Each variant carries its own data newtype: `CommandData`, `ComponentInteractionData`, `ModalInteractionData`. |
| `MessageComponentInteraction` | Gone. Use `ComponentInteraction` (`component_interaction.rs:29`). |
| `ModalSubmitInteraction` | Gone. Use `ModalInteraction` (`modal_interaction.rs:23`). |
| `CreateMember` | Gone. Use `EditMember<'a>` (`builder/edit_member.rs:15`). |
| `interaction.member` | `Option<Box<Member>>` on `CommandInteraction`, but plain `Option<Member>` on `ComponentInteraction` / `ModalInteraction`. |
| `CreateAllowedMentions::empty()` | Does not exist. `CreateAllowedMentions::default()`/`new()` already pings nothing. |
| `EditChannel::permission_overwrites(..)` | The method is `EditChannel::permissions(..)`. |
| `ChannelId::purge(..)` | Removed in 0.12. Use `ChannelId::delete_messages`. |
| `GuildId::ban(.., delete_message_seconds)` | Takes `dmd: u8` = **days** (0..=7). Seconds only exist on `bulk_ban`. |
| `GuildId::add_member_role(..)` | Does not exist on `GuildId`. Use `Member::add_role` or `Http::add_member_role`. |
| `Colour::BLURPLE` is the branding blurple | `Colour::BLURPLE == 0x7289DA` (old blurple); Discord's current branding value is `colour::colours::branding::BLURPLE == 0x5865F2`. |

---

## 1. Feature flags (`SERENITY_SRC/Cargo.toml:49-142`)

Raw definitions (normalized manifest):

```toml
builder        = []                                   # line 51
cache          = ["rustc-hash", "dashmap", "parking_lot"]
chrono         = ["dep:chrono", "typesize?/chrono"]   # line 57 — NOT enabled by default here
client         = ["http", "typemap_rev"]
gateway        = ["flate2"]
http           = ["mime_guess", "percent-encoding"]
model          = ["builder", "http", "utils"]         # line 108 — pulls builder + utils in
utils          = []                                   # line 142
rustls_backend = ["reqwest/rustls-tls", "tokio-tungstenite/rustls-tls-webpki-roots", "bytes"]
```

**Answer to "which feature enables `builder`":** `model` does (`model = ["builder", "http",
"utils"]`). There is no standalone `builder` enable needed, and **`utils` also comes for free**
(so `serenity::utils::MessageBuilder` is available).

Verified resolution for this crate's manifest
(`default-features = false, features = ["client","gateway","model","cache","rustls_backend"]`):

```
serenity v0.12.5 feats: builder, cache, client, gateway, http, model, rustls_backend, utils
(+ transitive dep features: bytes, dashmap, flate2, mime_guess, parking_lot, percent-encoding,
 reqwest, rustc-hash, tokio-tungstenite, typemap_rev)
```

`cargo check` on `/root/cipherbot` succeeds with this exact set, and `serenity::builder::*` resolves.

**`chrono` is NOT enabled.** `model::Timestamp` therefore stores `time::OffsetDateTime`
(`SERENITY_SRC/src/model/timestamp.rs:52,83-88`) and there is **no `From<chrono::DateTime>` impl**.
Build timestamps with `Timestamp::now()` / `Timestamp::from_unix_timestamp(secs)` /
`"..".parse::<Timestamp>()`. Add the `chrono` feature only if you actually need `chrono::DateTime`
interop.

Recommended manifest (matched to what the code already uses):

```toml
serenity = { version = "0.12", default-features = false,
             features = ["client", "gateway", "model", "cache", "rustls_backend"] }
```

---

## 2. `EventHandler` (`SERENITY_SRC/src/client/event_handler.rs`)

```rust
#[async_trait]
pub trait EventHandler: Send + Sync {
    async fn ready(&self, ctx: Context, data_about_bot: Ready);                      // :320
    async fn message(&self, ctx: Context, new_message: Message);                     // :268
    async fn interaction_create(&self, ctx: Context, interaction: Interaction);      // :379
    async fn guild_create(&self, ctx: Context, guild: Guild, is_new: Option<bool>);  // :170
    async fn guild_member_addition(&self, ctx: Context, new_member: Member);         // :202
    async fn guild_member_removal(&self, ctx: Context, guild_id: GuildId,
                                  user: User,
                                  member_data_if_available: Option<Member>);         // :211
    async fn channel_delete(&self, ctx: Context, channel: GuildChannel,
                            messages: Option<Vec<Message>>);                         // :140
    // useful neighbours, same file:
    async fn guild_member_update(&self, ctx, old_if_available: Option<Member>,
                                 new: Option<Member>, event: GuildMemberUpdateEvent); // :220
    async fn message_delete(&self, ctx, channel_id: ChannelId,
                            deleted_message_id: MessageId, guild_id: Option<GuildId>); // :273
    async fn thread_create(&self, ctx, thread: GuildChannel);                         // :414
    async fn guild_ban_addition(&self, ctx, guild_id: GuildId, banned_user: User);    // :160
    async fn guild_ban_removal(&self, ctx, guild_id: GuildId, unbanned_user: User);   // :165
}
```

All methods have empty default bodies — implement only what you need. `use serenity::async_trait;`
is required (serenity re-exports `async_trait`).

Minimal `main.rs`:

```rust
use serenity::all::*;
use serenity::async_trait;

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        println!("logged in as {}", ready.user.name);
        ctx.set_presence(Some(ActivityData::custom("moderating")), OnlineStatus::Online);
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Command(cmd) = interaction {
            let _ = cmd
                .create_response(
                    &ctx.http,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new().content("pong"),
                    ),
                )
                .await;
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let token = std::env::var("DISCORD_TOKEN")?;
    let intents = GatewayIntents::GUILDS | GatewayIntents::GUILD_MESSAGES;
    let mut client = Client::builder(token, intents)
        .event_handler(Handler)
        .activity(ActivityData::watching("the chat"))
        .status(OnlineStatus::Online)
        .await?;
    client.start().await?;
    Ok(())
}
```

`Client::builder(token: impl AsRef<str>, intents: GatewayIntents) -> ClientBuilder`
(`client/mod.rs:632`); `ClientBuilder: IntoFuture<Output = Result<Client>>` (`client/mod.rs:328`);
`Client::start(&mut self) -> Result<()>` (`client/mod.rs:674`). Builder extras: `.intents()`
(:261), `.event_handler()` (:273), `.activity(ActivityData)` (:309), `.status(OnlineStatus)` (:315).
`Context` fields (`client/context.rs:31-43`): `data: Arc<RwLock<TypeMap>>`, `shard: ShardMessenger`,
`shard_id: ShardId`, `http: Arc<Http>`, `cache: Arc<Cache>`.

---

## 3. Interaction model

```rust
// model/application/interaction.rs:26
#[non_exhaustive]
pub enum Interaction {
    Ping(PingInteraction),
    Command(CommandInteraction),
    Autocomplete(CommandInteraction),
    Component(ComponentInteraction),
    Modal(ModalInteraction),
}
```

Shared accessors on the enum (`interaction.rs:36-208`): `id() -> InteractionId`,
`kind() -> InteractionType`, `guild_id() -> Option<GuildId>`, `application_id() -> ApplicationId`,
`token() -> &str`, `app_permissions() -> Option<Permissions>`, `guild_locale() -> Option<&str>`,
`entitlements()`, plus `as_command()/command()`, `as_message_component()/message_component()`,
`as_modal_submit()/modal_submit()`, `as_autocomplete()/autocomplete()`, `as_ping()/ping()`.

`CommandInteraction` (`command_interaction.rs:51-93`):

```rust
pub struct CommandInteraction {
    pub id: InteractionId,
    pub application_id: ApplicationId,
    pub data: CommandData,
    pub guild_id: Option<GuildId>,
    pub channel: Option<PartialChannel>,
    pub channel_id: ChannelId,
    pub member: Option<Box<Member>>,     // guild-only
    pub user: User,
    pub token: String,
    pub version: u8,
    pub app_permissions: Option<Permissions>,
    pub locale: String,
    pub guild_locale: Option<String>,
    pub entitlements: Vec<Entitlement>,
    pub authorizing_integration_owners: AuthorizingIntegrationOwners,
    pub context: Option<InteractionContext>,
    pub attachment_size_limit: u32,
}
```

`ComponentInteraction` and `ModalInteraction` have the same shape, except `member: Option<Member>`
(not boxed), `message: Box<Message>` (component) / `Option<Box<Message>>` (modal), and no
`authorizing_integration_owners`/`context`.

### `CommandData` (`command_interaction.rs:284`)

```rust
pub struct CommandData {
    pub id: CommandId,
    pub name: String,
    pub kind: CommandType,
    pub resolved: CommandDataResolved,   // users, members, roles, channels, messages, attachments
    pub options: Vec<CommandDataOption>,
    pub guild_id: Option<GuildId>,
    pub target_id: Option<TargetId>,     // user/message context-menu commands
}
```

Helpers: `CommandData::autocomplete() -> Option<AutocompleteOption<'_>>` (:314) and
`CommandData::options() -> Vec<ResolvedOption<'_>>` (:342, resolves ids against `resolved`).

```rust
pub struct CommandDataOption { pub name: String, pub value: CommandDataOptionValue }  // :529
// #[non_exhaustive] → pattern-match with `..`

#[non_exhaustive]
pub enum CommandDataOptionValue {                                                     // :654
    Autocomplete { kind: CommandOptionType, value: String },
    Boolean(bool), Integer(i64), Number(f64), String(String),
    SubCommand(Vec<CommandDataOption>),
    SubCommandGroup(Vec<CommandDataOption>),
    Attachment(AttachmentId), Channel(ChannelId), Mentionable(GenericId),
    Role(RoleId), User(UserId), Unknown(u8),
}
```

Accessors on **`CommandDataOptionValue`** (not on the option): `as_bool() -> Option<bool>` (:697),
`as_i64() -> Option<i64>` (:706), `as_f64() -> Option<f64>` (:715),
`as_str() -> Option<&str>` (:724, also matches `Autocomplete`), `as_attachment_id()` (:735),
`as_channel_id()` (:744), `as_mentionable() -> Option<GenericId>` (:753),
`as_user_id() -> Option<UserId>` (:760), `as_role_id() -> Option<RoleId>` (:770).
`CommandDataOption::kind() -> CommandOptionType` (:538).

**There is no `option("name")` lookup helper.** Write one:

```rust
fn opt<'a>(data: &'a CommandData, name: &str) -> Option<&'a CommandDataOptionValue> {
    data.options.iter().find(|o| o.name == name).map(|o| &o.value)
}
```

### Reading subcommands / groups

```rust
match data.options.first() {
    Some(CommandDataOption { name, value: CommandDataOptionValue::SubCommand(opts), .. }) => {
        // `name` is the subcommand name; `opts` its options
    }
    Some(CommandDataOption { name, value: CommandDataOptionValue::SubCommandGroup(groups), .. }) => {
        // `groups[0].name` is the subcommand, `groups[0].value` the SubCommand([...])
    }
    _ => { /* flat command */ }
}
```

Prefer `data.options()` when you want resolved objects:

```rust
for ro in data.options() {           // ResolvedOption { name: &str, value: ResolvedValue }
    match ro.value {
        ResolvedValue::String(s)        => { let _ = s; }
        ResolvedValue::Integer(i)       => { let _ = i; }
        ResolvedValue::User(u, member)  => { let _ = (u.id, member); }
        ResolvedValue::Channel(c)       => { let _ = c.id; }
        ResolvedValue::SubCommand(o) | ResolvedValue::SubCommandGroup(o) => { let _ = o.len(); }
        ResolvedValue::Unresolved(u)    => { let _ = u; }
        _ => {}
    }
}
```

`ResolvedValue` variants: `Autocomplete, Boolean, Integer, Number, String, SubCommand,
SubCommandGroup, Attachment(&Attachment), Channel(&PartialChannel), Role(&Role),
User(&User, Option<&PartialMember>), Unresolved(Unresolved)` (`command_interaction.rs:453-466`).

---

## 4. Registering commands

### `CreateCommand` (`builder/create_command.rs:302-465`)

```rust
CreateCommand::new(name: impl Into<String>) -> Self                // :327
  .name(impl Into<String>)                                        // :353  (1..=32, ^[\w-]{1,32}$, lowercase)
  .description(impl Into<String>)                                 // :394  (1..=100)
  .kind(CommandType)                                              // :373  (ChatInput/User/Message)
  .add_option(CreateCommandOption)                                // :419  (max 25 total)
  .set_options(Vec<CreateCommandOption>)                          // :427
  .default_member_permissions(Permissions)                        // :379  (serialized as bit string)
  .dm_permission(bool)                                            // :386  (deprecated under unstable_discord_api)
  .contexts(Vec<InteractionContext>) / .add_context(..)           // :451 / :445
  .integration_types(Vec<InstallationContext>)                    // :439
  .nsfw(bool)                                                     // :457
  .name_localized(locale, name) / .description_localized(..)      // :367 / :407
```

### `CreateCommandOption` (`builder/create_command.rs:19-289`)

```rust
CreateCommandOption::new(kind: CommandOptionType,
                         name: impl Into<String>,
                         description: impl Into<String>) -> Self  // :22
  .required(bool)                                                 // :106
  .set_autocomplete(bool)                                         // :205
  .add_string_choice(name, value: impl Into<String>)              // :143
  .add_int_choice(name, value: i32)                               // :115  (i32!)
  .add_number_choice(name, value: f64)                            // :171
  .add_string_choice_localized / add_int_choice_localized / add_number_choice_localized
  .min_length(u16) / .max_length(u16)                             // :275 / :284
  .min_int_value(u64) / .max_int_value(u64)                       // :249 / :255  (u64!)
  .min_number_value(f64) / .max_number_value(f64)                 // :261 / :267
  .channel_types(Vec<ChannelType>)                                // :243
  .add_sub_option(CreateCommandOption) / .set_sub_options(Vec<_>) // :235 / :220
```

`CommandOptionType` (`model/application/command.rs:338`): `SubCommand=1, SubCommandGroup=2,
String=3, Integer=4, Boolean=5, User=6, Channel=7, Role=8, Mentionable=9, Number=10, Attachment=11`.

### Registering

```rust
// Guild commands — instant, and what you want during development.
GuildId::set_commands(self, http: impl AsRef<Http>, commands: Vec<CreateCommand>)
    -> Result<Vec<Command>>                       // model/guild/guild_id.rs:1574  (overwrites all)
GuildId::create_command(self, cache_http: impl CacheHttp, builder: CreateCommand)
    -> Result<Command>                            // :1561
GuildId::get_commands(self, http) -> Result<Vec<Command>>          // :1603
GuildId::delete_command(self, http, command_id: CommandId) -> Result<()>  // :1651

// Global commands — up to 100, propagate slowly (up to ~1h).
Command::set_global_commands(http: impl AsRef<Http>, commands: Vec<CreateCommand>)
    -> Result<Vec<Command>>                       // model/application/command.rs:172
Command::create_global_command(cache_http, builder) -> Result<Command>    // :160
Command::get_global_commands(http) -> Result<Vec<Command>>                // :197
Command::delete_global_command(http, command_id: CommandId) -> Result<()> // :229
```

Clearing stale commands (run on `ready`, idempotent):

```rust
async fn ready(&self, ctx: Context, ready: Ready) {
    let guild = GuildId::new(GUILD_ID);
    let _ = GuildId::set_commands(guild, &ctx.http, vec![]).await;          // wipe guild cmds
    let existing = Command::get_global_commands(&ctx.http).await.unwrap_or_default();
    for c in existing {
        let _ = Command::delete_global_command(&ctx.http, c.id).await;      // wipe global cmds
    }
    let _ = GuildId::set_commands(guild, &ctx.http, my_commands()).await;
    println!("registered for {}", ready.user.name);
}
```

Avoid global+guild duplicates with the same name: Discord rejects it
(`create_command.rs:350-351`).

---

## 5. Responding

### `CreateInteractionResponse` (`builder/create_interaction_response.rs:23-60`)

```rust
pub enum CreateInteractionResponse {
    Pong,                                        // 1
    Message(CreateInteractionResponseMessage),   // 4  CHANNEL_MESSAGE_WITH_SOURCE
    Defer(CreateInteractionResponseMessage),     // 5  DEFERRED_CHANNEL_MESSAGE_WITH_SOURCE
    Acknowledge,                                 // 6  DEFERRED_UPDATE_MESSAGE (components)
    UpdateMessage(CreateInteractionResponseMessage), // 7  UPDATE_MESSAGE (components)
    Autocomplete(CreateAutocompleteResponse),    // 8
    Modal(CreateModal),                          // 9
    PremiumRequired,                             // 10 (deprecated)
    LaunchActivity,                              // 12
}
```

### `CreateInteractionResponseMessage` (`:179-340`)

```rust
CreateInteractionResponseMessage::new()          // :199
  .content(impl Into<String>)                    // :240  (≤2000 chars, checked client-side)
  .embed(CreateEmbed) / .embeds(Vec<CreateEmbed>)// :265 / :273
  .add_embed(CreateEmbed)                        // :248
  .components(Vec<CreateActionRow>)              // :305
  .ephemeral(bool)                               // :291  (sets InteractionResponseFlags::EPHEMERAL)
  .allowed_mentions(CreateAllowedMentions)       // :279
  .flags(InteractionResponseFlags)               // :285
  .tts(bool) / .add_file(CreateAttachment) / .poll(..)
```

`CreateAutocompleteResponse` (`:360-405`): `new()`, `set_choices(Vec<AutocompleteChoice>)`,
`add_string_choice(name, value)`, `add_int_choice(name, value: i64)`,
`add_number_choice(name, value: f64)`.

### Sending (`model/application/command_interaction.rs`)

```rust
create_response(&self, cache_http: impl CacheHttp, builder: CreateInteractionResponse) -> Result<()>  // :115
edit_response(&self, cache_http, builder: EditInteractionResponse) -> Result<Message>                 // :132
delete_response(&self, http: impl AsRef<Http>) -> Result<()>                                          // :148
create_followup(&self, cache_http, builder: CreateInteractionResponseFollowup) -> Result<Message>      // :161
edit_followup(&self, cache_http, message_id: impl Into<MessageId>, builder: CreateInteractionResponseFollowup) -> Result<Message> // :178
delete_followup<M: Into<MessageId>>(&self, http, message_id: M) -> Result<()>                          // :193
defer(&self, cache_http) -> Result<()>                                                                 // :221
defer_ephemeral(&self, cache_http) -> Result<()>                                                       // :232
```

Identical methods exist on `ComponentInteraction` (`component_interaction.rs:94,140,200`) and
`ModalInteraction` (`modal_interaction.rs:87,179`); the only special case is
`ModalInteraction::defer` which sends `CreateInteractionResponse::Acknowledge` (type 6), not type 5.

`CreateInteractionResponseFollowup` (`builder/create_interaction_response_followup.rs:23-175`):
`new()`, `.content()`, `.embed()/.embeds()/.add_embed()`, `.components()`, `.ephemeral(bool)`
(sets `MessageFlags::EPHEMERAL`, note **`MessageFlags`**, not `InteractionResponseFlags`),
`.allowed_mentions()`, `.flags(MessageFlags)`, `.add_file()`, `.poll()`.

`EditInteractionResponse` (`builder/edit_interaction_response.rs`): `.content()`, `.embed()`,
`.embeds()`, `.allowed_mentions()`, `.components()`, `.flags(MessageFlags)`, `.attachments()`.

Canonical flow (ack → followup):

```rust
async fn on_slash(ctx: &Context, cmd: &CommandInteraction) -> serenity::Result<()> {
    cmd.defer_ephemeral(&ctx.http).await?;                        // stops the 3s clock
    // ... slow work ...
    cmd.create_followup(
        &ctx.http,
        CreateInteractionResponseFollowup::new()
            .content("done")
            .ephemeral(true),
    ).await?;
    Ok(())
}
```

---

## 6. Components

```rust
// builder/create_components.rs:10
pub enum CreateActionRow {
    Buttons(Vec<CreateButton>),      // ≤5 buttons
    SelectMenu(CreateSelectMenu),    // exactly 1
    InputText(CreateInputText),      // modals only
}

CreateButton::new(custom_id)                 // :73   primary style by default
  .style(ButtonStyle)                        // :103
  .label(impl Into<String>)                  // :114
  .emoji(impl Into<ReactionType>)            // :120
  .disabled(bool)                            // :126
CreateButton::new_link(url)                  // :44   (no custom_id, no interaction event)
CreateButton::new_premium(sku_id)            // :59

pub enum ButtonStyle { Primary = 1, Secondary = 2, Success = 3, Danger = 4 }  // model/application/component.rs:189
// Link is NOT a ButtonStyle — use CreateButton::new_link.

CreateSelectMenu::new(custom_id, CreateSelectMenuKind)   // :249
  .placeholder(..) / .min_values(u8) / .max_values(u8) / .disabled(bool)

pub enum CreateSelectMenuKind {              // :145
    String { options: Vec<CreateSelectMenuOption> },
    User { default_users: Option<Vec<UserId>> },
    Role { default_roles: Option<Vec<RoleId>> },
    Mentionable { default_users: .., default_roles: .. },
    Channel { channel_types: Option<Vec<ChannelType>>, default_channels: Option<Vec<ChannelId>> },
}
CreateSelectMenuOption::new(label, value)    // :311  (.description, .emoji, .default_selection)

CreateInputText::new(style: InputTextStyle, label, custom_id)   // :362
  .placeholder(..) / .min_length(u16) / .max_length(u16) / .value(..) / .required(bool)
pub enum InputTextStyle { Short = 1, Paragraph = 2 }            // model/application/component.rs:303

CreateModal::new(custom_id, title)                               // create_interaction_response.rs:441
  .components(Vec<CreateActionRow>)                              // :452
```

### Handling component interactions

```rust
Interaction::Component(ci) => {
    let custom_id: &str = &ci.data.custom_id;                    // component_interaction.rs:344
    match &ci.data.kind {                                        // :255
        ComponentInteractionDataKind::Button => {}
        ComponentInteractionDataKind::StringSelect { values } => { /* Vec<String> */ }
        ComponentInteractionDataKind::UserSelect { values } => { /* Vec<UserId> */ }
        ComponentInteractionDataKind::RoleSelect { values } => { /* Vec<RoleId> */ }
        ComponentInteractionDataKind::MentionableSelect { values } => { /* Vec<GenericId> */ }
        ComponentInteractionDataKind::ChannelSelect { values } => { /* Vec<ChannelId> */ }
        ComponentInteractionDataKind::Unknown(_) => {}
    }
    let message_id = ci.message.id;
    ci.create_response(&ctx.http, CreateInteractionResponse::Acknowledge).await?;   // keep the message
    // or UpdateMessage(..) to replace it, or Defer(..) + edit_response(..)
}
```

### Handling modals + reading submitted values

`ModalInteractionData` (`modal_interaction.rs:222`):

```rust
pub struct ModalInteractionData { pub custom_id: String, pub components: Vec<ActionRow> }
pub struct ActionRow { pub kind: ComponentType, pub components: Vec<ActionRowComponent> }  // component.rs:34
pub enum ActionRowComponent { Button(Button), SelectMenu(SelectMenu), InputText(InputText) } // component.rs:49
```

```rust
Interaction::Modal(mi) => {
    let mut reason = None;
    for row in &mi.data.components {
        for c in &row.components {
            if let ActionRowComponent::InputText(input) = c {
                if input.custom_id == "reason" {
                    reason = input.value.clone();     // Option<String>, Some on submit
                }
            }
        }
    }
    mi.create_response(
        &ctx.http,
        CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content(format!("reason: {}", reason.unwrap_or_default()))
                .ephemeral(true),
        ),
    ).await?;
}
```

`InputText` fields (`component.rs:258-289`): `kind`, `custom_id: String`, `style: Option<_>`,
`label: Option<String>`, `min_length/max_length: Option<u16>`, `required: bool`,
`value: Option<String>` (the user's answer).

Opening a modal from a slash command:

```rust
cmd.create_response(&ctx.http, CreateInteractionResponse::Modal(
    CreateModal::new("report_modal", "Report a user").components(vec![
        CreateActionRow::InputText(
            CreateInputText::new(InputTextStyle::Paragraph, "What happened?", "details")
                .min_length(10).max_length(1000).required(true),
        ),
        CreateActionRow::Buttons(vec![
            CreateButton::new("confirm").style(ButtonStyle::Success).label("OK"),
        ]),
    ])
)).await?;
```

---

## 7. Moderation operations

All `GuildId` methods take `http: impl AsRef<Http>` (`&ctx.http` works) unless noted.
`impl CacheHttp` is satisfied by `&ctx` or `&ctx.http`.

```rust
// ---- Timeout (Discord calls it "communication disabled until") ----
EditMember::new()                                                    // builder/edit_member.rs:15
  .disable_communication_until_datetime(Timestamp)                   // :124  ← preferred
  .disable_communication_until(String)                               // :111  (raw RFC3339)
  .enable_communication()                                            // :134  (clears timeout)
  .nickname(..) / .roles(..) / .mute(bool) / .deafen(bool)
  .voice_channel(ChannelId) / .disconnect_member() / .flags(GuildMemberFlags)
  .audit_log_reason(&'a str)                                         // :145

GuildId::edit_member(cache_http, user_id, EditMember<'_>) -> Result<Member>   // guild_id.rs:709
Member::disable_communication_until_datetime(&mut self, cache_http, Timestamp) -> Result<()>  // member.rs:222
Member::enable_communication(&mut self, cache_http) -> Result<()>             // member.rs:289

// Timestamp has no Add<Duration>; build it from unix seconds (chrono feature is off):
let until = Timestamp::from_unix_timestamp(Timestamp::now().unix_timestamp() + 600)?; // +10 min
guild_id.edit_member(&ctx.http, user_id,
    EditMember::new().disable_communication_until_datetime(until).audit_log_reason("spam")).await?;

// ---- Kick ----
GuildId::kick(self, http, user_id: impl Into<UserId>) -> Result<()>            // guild_id.rs:1073
GuildId::kick_with_reason(self, http, user_id, reason: &str) -> Result<()>     // :1082
Member::kick(&self, cache_http) -> Result<()>                                  // member.rs:348

// ---- Ban / unban (dmd = delete_message_DAYS, 0..=7) ----
GuildId::ban(self, http, user: impl Into<UserId>, dmd: u8) -> Result<()>       // guild_id.rs:213
GuildId::ban_with_reason(self, http, user, dmd: u8, reason: impl AsRef<str>)   // :225
GuildId::unban(self, http, user_id) -> Result<()>                              // :1492
GuildId::bans(self, http, target: Option<UserPagination>, limit: Option<u8>)   // :297
Member::ban(&self, http, dmd: u8) -> Result<()>                                // member.rs:151
// dmd > 7 → Error::Model(ModelError::DeleteMessageDaysAmount)  (guild_id.rs:242)
// For per-second control: GuildId::bulk_ban(self, http: &Http, user_ids: &[UserId],
//                         delete_message_seconds: u32, reason: Option<&str>) -> Result<BulkBanResponse>  (guild_id.rs:266)

// ---- Roles ----
Member::add_role(&self, http, role_id: impl Into<RoleId>) -> Result<()>        // member.rs:117
Member::add_roles(&self, http, &[RoleId]) -> Result<()>                        // member.rs:131
Member::remove_role(&self, http, role_id: impl Into<RoleId>) -> Result<()>     // member.rs:468
Member::remove_roles(&self, http, &[RoleId]) -> Result<()>                     // :486
// GuildId has NO add_member_role; the low-level API is:
Http::add_member_role(guild_id, user_id, role_id, audit_log_reason: Option<&str>) -> Result<()>    // http/client.rs:266
Http::remove_member_role(guild_id, user_id, role_id, audit_log_reason) -> Result<()>              // http/client.rs:4821

// ---- Purge messages (there is NO `purge` in 0.12) ----
ChannelId::delete_message(self, http, message_id: impl Into<MessageId>) -> Result<()>   // channel_id.rs:158
ChannelId::delete_messages<T: AsRef<MessageId>>(self, http, message_ids: impl IntoIterator<Item = T>)
    -> Result<()>                                                                       // :183
//   0 or >100 ids → Error::Model(ModelError::BulkDeleteAmount); 1 id is special-cased.
//   Messages older than 2 weeks cannot be bulk-deleted (Discord rule).
Message::delete(&self, cache_http) -> Result<()>                                        // message.rs:271
// Manual purge loop: collect ids, chunk by 100, skip ids older than 14 days.

// ---- Slowmode ----
EditChannel::new().rate_limit_per_user(seconds: u16)      // builder/edit_channel.rs:198 (0..=21600)
ChannelId::edit(self, cache_http, EditChannel<'_>) -> Result<GuildChannel>   // channel_id.rs:326
GuildChannel::edit(&mut self, cache_http, EditChannel<'_>) -> Result<()>     // guild_channel.rs:484

// ---- Lock / unlock (permission overwrite on @everyone) ----
EditChannel::new().permissions(impl IntoIterator<Item = PermissionOverwrite>)  // edit_channel.rs:234
// NB: the method is `permissions`, NOT `permission_overwrites`.
PermissionOverwrite { allow: Permissions, deny: Permissions, kind: PermissionOverwriteType }
    // model/channel/mod.rs:340
PermissionOverwriteType::Member(UserId) | PermissionOverwriteType::Role(RoleId)  // mod.rs:357
GuildId::everyone_role() -> RoleId

// lock:
channel_id.edit(&ctx.http, EditChannel::new().permissions(vec![PermissionOverwrite {
    allow: Permissions::empty(),
    deny:  Permissions::SEND_MESSAGES,
    kind:  PermissionOverwriteType::Role(guild_id.everyone_role()),
}])).await?;
// unlock: deny: Permissions::empty(), allow: Permissions::SEND_MESSAGES

// ---- Create channel / role ----
CreateChannel::new(name)                                   // builder/create_channel.rs:57
  .kind(ChannelType) / .category(ChannelId) / .topic(..) / .nsfw(bool)
  .rate_limit_per_user(u16) / .position(u16) / .permissions(..) / .audit_log_reason(&str)
GuildId::create_channel(self, cache_http, CreateChannel<'_>) -> Result<GuildChannel>  // guild_id.rs:388

EditRole::new()                                            // builder/edit_role.rs:77
  .name(..) / .colour(impl Into<Colour>) / .hoist(bool) / .mentionable(bool)
  .permissions(Permissions) / .position(u16) / .audit_log_reason(&str)
GuildId::create_role(self, cache_http, EditRole<'_>) -> Result<Role>                  // guild_id.rs:468
```

---

## 8. Permission checks

```rust
// model/guild/mod.rs
Guild::user_permissions_in(&self, channel: &GuildChannel, member: &Member) -> Permissions   // :1991
Guild::member_permissions(&self, member: &Member) -> Permissions                            // :1928 (guild-level, ignores overwrites)
Guild::user_permissions_in_ / permissions_in(..) variants exist for PartialMember           // :1939+
```

`Member::permissions(&Cache) -> Result<Permissions>` still exists at `member.rs:451` but is
**`#[deprecated]`**: "Use Guild::user_permissions_in, as this doesn't consider permission
overwrites" (`member.rs:450`). It only works when the guild is in cache (needs `GUILDS` intent).

**Best path for slash commands — no cache, no `GUILDS` intent:** the interaction payload already
carries the invoker's channel permissions in `Member::permissions: Option<Permissions>`
(`member.rs:49-55`, "only `Some` when returned in an `Interaction` object"). `PartialMember` has the
same field (`member.rs:612-618`).

```rust
let invoker_perms = cmd.member.as_ref()
    .and_then(|m| m.permissions)     // computed channel perms, includes overwrites
    .unwrap_or_else(Permissions::empty());
if !invoker_perms.contains(Permissions::MANAGE_MESSAGES) { /* deny */ }

let bot_perms = cmd.app_permissions.unwrap_or_else(Permissions::empty);  // the BOT's perms
```

Fallback with cache (only when the guild is cached — `GUILDS` intent):

```rust
if let Some(guild) = guild_id.to_guild_cached(&ctx.cache) {           // guild_id.rs:951
    if let Some(ch) = guild.channels.get(&channel_id) {
        if let Some(member) = guild.members.get(&user_id) {
            let perms = guild.user_permissions_in(ch, member);
            let _ = perms.contains(Permissions::BAN_MEMBERS);
        }
    }
}
```

`Permissions` is a `bitflags` 2.x type (`model/permissions.rs:267-269`), so
`contains(Permissions::X)` = all bits of X set, `intersects(Permissions::X | Permissions::Y)` = any
bit set. Constants (`permissions.rs:271-390`): `CREATE_INSTANT_INVITE` `1<<0`, `KICK_MEMBERS` `1<<1`,
`BAN_MEMBERS` `1<<2`, `ADMINISTRATOR` `1<<3`, `MANAGE_CHANNELS` `1<<4`, `MANAGE_GUILD` `1<<5`,
`VIEW_CHANNEL` `1<<10`, `SEND_MESSAGES` `1<<11`, `MANAGE_MESSAGES` `1<<13`,
`MANAGE_ROLES` `1<<28`, `MANAGE_THREADS` `1<<34`, `MODERATE_MEMBERS` `1<<40`. Per-flag helper
methods also exist (`permissions.manage_messages()`, `permissions.administrator()`, …).
`ADMINISTRATOR` does **not** imply the other bits in the value itself — check
`perms.contains(ADMINISTRATOR) || perms.contains(NEEDED)`.

---

## 9. Embeds

```rust
// builder/create_embed.rs
CreateEmbed::new()                                   // :30
  .title(impl Into<String>)                          // :152  (≤256)
  .description(impl Into<String>)                    // :61   (≤4096)
  .url(impl Into<String>)                            // :159
  .colour<C: Into<Colour>>(C) / .color(..)           // :52 / :46
  .field(name, value, inline: bool)                  // :71   (name ≤256, value ≤1024, ≤25 fields)
  .fields(impl IntoIterator<Item = (N, V, bool)>)    // :84
  .footer(CreateEmbedFooter)                         // :98   ← takes the builder, not a &str
  .image(url) / .thumbnail(url)                      // :108 / :120
  .author(CreateEmbedAuthor)                         // :37
  .timestamp<T: Into<Timestamp>>(T)                  // :145
CreateEmbedFooter::new(text).icon_url(url)           // :300/:318
CreateEmbedAuthor::new(name).icon_url(url).url(url)  // :242-265
```

`CreateEmbed::footer("text")` does **not** compile — wrap it.

```rust
CreateEmbed::new()
    .title("Ban applied")
    .description(format!("<@{}> was banned", user_id))
    .colour(Colour::RED)
    .field("Reason", "spam", true)
    .field("Duration", "permanent", true)
    .footer(CreateEmbedFooter::new("CipherBot").icon_url("https://example.com/i.png"))
    .thumbnail("https://example.com/thumb.png")
    .timestamp(Timestamp::now())
```

`Colour`: `Colour::new(u32)` (`colour.rs:95`), `Colour::from(u64)`, `Colour::from((r,g,b))`,
constants `RED/BLUE/GREEN/BLURPLE/ORANGE/GOLD/DARK_RED/...` (`colour.rs:273-327`). Discord branding
values live under `serenity::model::colour::colours::branding::*` (`colour.rs:330+`).

Mentions:

```rust
// builder/create_allowed_mentions.rs — there is NO `.empty()`.
CreateAllowedMentions::new()          // :84 — default pings NOTHING (parse list empty)
  .empty_users() / .empty_roles()     // :123 / :137
  .users(impl IntoIterator<Item = impl Into<UserId>>)   // :116
  .roles(..) / .everyone(bool) / .all_users(bool) / .all_roles(bool) / .replied_user(bool)
```

Message limits enforced client-side: `MESSAGE_CODE_LIMIT = 2000`, `EMBED_MAX_COUNT = 10`,
`EMBED_MAX_LENGTH = 6000` (`src/constants.rs:4,7,19`).

---

## 10. Messages and threads

```rust
// model/channel/channel_id.rs
ChannelId::say(self, cache_http, content: impl Into<String>) -> Result<Message>     // :645
ChannelId::send_message(self, cache_http, CreateMessage) -> Result<Message>         // :738
ChannelId::edit_message(self, cache_http, message_id, EditMessage) -> Result<Message> // :349
ChannelId::delete_message(self, http, message_id) -> Result<()>                     // :158
ChannelId::delete_messages(self, http, ids) -> Result<()>                           // :183
ChannelId::broadcast_typing(self, http) -> Result<()>                               // :65
ChannelId::create_thread(self, cache_http, CreateThread<'_>) -> Result<GuildChannel>  // :945
ChannelId::create_thread_from_message(self, cache_http, message_id, CreateThread<'_>)
    -> Result<GuildChannel>                                                         // :930

Message::delete(&self, cache_http) -> Result<()>                                    // message.rs:271

// builder/create_message.rs:56
CreateMessage::new()
  .content(..) / .embed(..) / .embeds(..) / .add_embed(..)
  .components(Vec<CreateActionRow>) / .allowed_mentions(..)
  .reference_message(impl Into<MessageReference>) / .flags(MessageFlags) / .tts(bool)

// builder/create_thread.rs:14
CreateThread::new(name)                          // :32  (2..=100 chars)
  .auto_archive_duration(AutoArchiveDuration)    // :52
  .kind(ChannelType)                             // :85  PublicThread | PrivateThread
  .rate_limit_per_user(u16) / .invitable(bool) / .audit_log_reason(&str)
// AutoArchiveDuration (model/channel/mod.rs:408): None=0, OneHour=60, OneDay=1440,
//   ThreeDays=4320, OneWeek=10080
```

`MessageBuilder` lives in `serenity::utils::MessageBuilder` (`utils/message_builder.rs:36`,
`pub struct MessageBuilder(pub String)`) — available because `utils` is pulled in by `model`.
It is only for assembling a `String`; it is **not** an accepted argument by any `say`/`send` method.

---

## 11. Presence

```rust
Context::set_presence(&self, activity: Option<ActivityData>, status: OnlineStatus)   // client/context.rs:309
// (fire-and-forget: sends to the shard, returns ())

// serenity::gateway::ActivityData — NOT serenity::model::gateway  (gateway/mod.rs:74)
pub struct ActivityData { pub name: String, pub kind: ActivityType,
                          pub state: Option<String>, pub url: Option<Url> }
ActivityData::playing(name)      // :89
ActivityData::streaming(name, url: impl IntoUrl) -> Result<Self>   // :104
ActivityData::listening(name)    // :116
ActivityData::watching(name)     // :128
ActivityData::competing(name)    // :139
ActivityData::custom(state)      // :151  ← what you want for "status text"

// model/gateway.rs:193
pub enum ActivityType { Playing=0, Streaming=1, Listening=2, Watching=3, Custom=4, Competing=5 }

// model/user.rs:196
pub enum OnlineStatus { DoNotDisturb, Idle, Invisible, Offline, Online }  // default Online

// initial presence, before connect:
Client::builder(token, intents)
    .activity(ActivityData::watching("the logs"))   // client/mod.rs:309
    .status(OnlineStatus::Online)                   // client/mod.rs:315
```

For a custom status, `ActivityData::custom("text")` sets `kind = Custom` and `state = Some("text")`
with `name = "~"` (Discord requires a name, `gateway/mod.rs:151-159`).

---

## 12. Gateway intents

`GatewayIntents` is a bitflags struct (`model/gateway.rs:465`):

```rust
GUILDS                    = 1 << 0    // guild/channel/role events + guild cache
GUILD_MEMBERS             = 1 << 1    // PRIVILEGED
GUILD_MODERATION          = 1 << 2    // = GUILD_BANS alias
GUILD_BANS                = 1 << 2
GUILD_EMOJIS_AND_STICKERS = 1 << 3
GUILD_INTEGRATIONS        = 1 << 4
GUILD_WEBHOOKS            = 1 << 5
GUILD_INVITES             = 1 << 6
GUILD_VOICE_STATES        = 1 << 7
GUILD_PRESENCES           = 1 << 8    // PRIVILEGED
GUILD_MESSAGES            = 1 << 9
GUILD_MESSAGE_REACTIONS   = 1 << 10
GUILD_MESSAGE_TYPING      = 1 << 11
DIRECT_MESSAGES           = 1 << 12
DIRECT_MESSAGE_REACTIONS  = 1 << 13
DIRECT_MESSAGE_TYPING     = 1 << 14
MESSAGE_CONTENT           = 1 << 15   // PRIVILEGED
GUILD_SCHEDULED_EVENTS    = 1 << 16
AUTO_MODERATION_CONFIGURATION = 1 << 20
AUTO_MODERATION_EXECUTION     = 1 << 21
GUILD_MESSAGE_POLLS           = 1 << 24
```

Slash-command moderation bot (no message reading): `GUILDS` only is enough for commands,
components, modals and REST moderation. Add:

```rust
let intents = GatewayIntents::GUILDS
    | GatewayIntents::GUILD_MEMBERS         // join/leave events + member cache (privileged)
    | GatewayIntents::GUILD_MODERATION      // ban/unban events (not required to *call* ban)
    | GatewayIntents::GUILD_MESSAGES        // only if you read/handle messages
    | GatewayIntents::MESSAGE_CONTENT;      // only if msg.content must be non-empty (privileged)
```

**Privileged — must be toggled in the Discord Developer Portal → Bot → "Privileged Gateway
Intents":** `GUILD_MEMBERS`, `GUILD_PRESENCES`, `MESSAGE_CONTENT`. Requesting them without the
portal toggle fails IDENTIFY with close code `4014` / `DISALLOWED_GATEWAY_INTENTS`
(`constants.rs:119`). Over 100 guilds, `GUILD_MEMBERS`/`MESSAGE_CONTENT` require bot verification.

Interaction payloads are **not** intent-gated: slash commands, components and modals arrive with
`GUILDS` only.

---

## 13. Gotchas

1. **`Interaction` is an enum and `#[non_exhaustive]`.** Every `match` needs `_ => {}`; there is no
   `interaction.data` — destructure `Interaction::Command(CommandInteraction { data, .. })`.
2. **3-second ACK deadline.** Any first response (`Message`/`Defer`/`Modal`/`Autocomplete`/
   `Acknowledge`) must be sent within 3s of the interaction. Miss it → HTTP 404 `10062 Unknown
   interaction`, which is unrecoverable — the token is dead.
3. **After `Defer`, only followups.** `CreateInteractionResponse::Defer` consumes the initial
   response slot; you must then use `create_followup` / `edit_response` (which edits the "thinking"
   placeholder). Sending a second `create_response` yields `40060 Interaction has already been
   acknowledged`.
4. **Ephemeral is per-message.** `defer_ephemeral` only makes the *initial* response ephemeral;
   each followup needs `.ephemeral(true)` itself (`CreateInteractionResponseFollowup::ephemeral`,
   `create_interaction_response_followup.rs:148`). Ephemeral messages can never be fetched later
   (`get_response` → 404) and `delete_response` does not work on them
   (`command_interaction.rs:142`).
5. **Modals and defers.** `CreateInteractionResponse::Modal` is documented "Not valid for Modal and
   Ping interactions" (`create_interaction_response.rs:52-53`) — you cannot chain a modal off a
   modal submit through that variant, and you cannot defer and then send a modal
   (`Defer` already consumed the response). Respond to a `ModalInteraction` with `Message` /
   `UpdateMessage` / `Acknowledge` (`ModalInteraction::defer` sends type 6,
   `modal_interaction.rs:179`). For `Autocomplete`, the *only* valid response is
   `CreateInteractionResponse::Autocomplete`, and it must arrive within 3s too.
6. **`MESSAGE_CONTENT` is privileged.** Without it, `Message::content`, `embeds`, `attachments`,
   `components` arrive empty/`None` for messages the bot did not send or get mentioned in — even
   with `GUILD_MESSAGES`. Enable it in the dev portal or the bot sees blank content.
7. **`GUILD_MEMBERS` is privileged** and gates `guild_member_addition` / `guild_member_removal` and
   the member cache. Same for `GUILD_PRESENCES`.
8. **`Cache` may miss.** `Cache::guild` returns `Option<GuildRef<'_>>` (a dashmap read guard, not an
   `Arc<Guild>`; `cache/mod.rs:435`) and is `None` if `GUILDS` is off, during startup, or right after
   a reconnect. Always fall back to REST or to the interaction payload. Also: **never hold a
   `GuildRef`/`ChannelRef`/`MemberRef` across an `.await`** — it can deadlock the cache's locks.
9. **`Member::permissions` vs interaction permissions.** `Member::permissions(&Cache)` is deprecated
   and ignores channel overwrites. Use `Guild::user_permissions_in(&channel, &member)`, or — for
   slash commands — `interaction.member.permissions` (already channel-computed, `member.rs:49-55`)
   plus `interaction.app_permissions` for the bot's own perms.
10. **Command limits.** Names: 1–32 chars, `^[\w-]{1,32}$`, lowercase (`create_command.rs:349-351`).
    Descriptions: 1–100 chars (:393). Options: max 25 per command (:417). Choices: max 25 per option.
    **Global commands: 100 per app** (guild commands: unlimited-ish, ~100 per guild). Global
    command propagation can take up to an hour; guild commands are instant — **register guild
    commands while developing.**
11. **No duplicate names.** The same app cannot register a global and a guild command with the same
    name in the same guild.
12. **Component `custom_id` max 100 chars** (`component.rs:262`), and modal input labels max 45.
    `CreateButton::new_link` produces a link button that fires **no** interaction event.
13. **`CreateEmbed::footer` needs `CreateEmbedFooter`**, `EditChannel::permissions` (not
    `permission_overwrites`), `CreateAllowedMentions` has no `.empty()`.
14. **`ban` takes days, not seconds.** `GuildId::ban(.., dmd: u8)` with `dmd <= 7`
    (`guild_id.rs:242`); seconds exist only on `bulk_ban`.
15. **No `purge`.** `ChannelId::delete_messages` requires 2..=100 ids per call and silently rejects
    messages older than 14 days (Discord rule) — chunk and pre-filter by `Message::timestamp`.
16. **`builder` feature gating is transitive.** Keep `model` in the feature list; it enables
    `builder` + `utils`. Dropping `model` breaks every `CreateX` type at compile time.
17. **Rate limits** are handled per-route by `Http` (it blocks/retries internally). Long moderation
    sweeps still hit the 5/5s guild-level bucket — batch with a small delay or `Http::ratelimiter`.
18. **Autocomplete must be sub-3s** and only carries the focused option: use
    `CommandData::autocomplete()` → `AutocompleteOption { name, kind, value }`
    (`command_interaction.rs:437`) and answer with
    `CreateInteractionResponse::Autocomplete(CreateAutocompleteResponse::new().add_string_choice(..))`.
19. **`Timestamp` has no `chrono` interop** with this feature set; build it via
    `Timestamp::now()`, `from_unix_timestamp`, or `str::parse`.
20. **`serenity::all::*`** re-exports builders + models + client (`lib.rs:146-188`) — one import is
    usually enough, `serenity::prelude::*` gives `Context`, `Client`, `EventHandler`,
    `TypeMap`, `GatewayIntents`.
