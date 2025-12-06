//! Permission management commands for playground projects.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use color_eyre::eyre::{Result, bail, eyre};
use serde::Serialize;

use crate::ui;
use waterui_cli::{
    output,
    permission::Permission,
    project::{Config, PackageType},
};

#[derive(Subcommand, Debug)]
pub enum PermissionCommands {
    /// Add a permission to the project
    Add(AddPermissionArgs),
    /// Remove a permission from the project
    Remove(RemovePermissionArgs),
    /// List all configured permissions
    List(ListPermissionsArgs),
    /// Show all available permissions
    Available(AvailablePermissionsArgs),
}

#[derive(Args, Debug)]
pub struct AddPermissionArgs {
    /// Permission to add (e.g., "camera", "location")
    pub permission: String,

    /// Custom usage description (used for iOS Info.plist)
    #[arg(long, short = 'd')]
    pub description: Option<String>,

    /// Project directory
    #[arg(long)]
    pub project: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct RemovePermissionArgs {
    /// Permission to remove
    pub permission: String,

    /// Project directory
    #[arg(long)]
    pub project: Option<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct ListPermissionsArgs {
    /// Project directory
    #[arg(long)]
    pub project: Option<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct AvailablePermissionsArgs {
    /// Filter by platform (ios, android)
    #[arg(long)]
    pub platform: Option<String>,
}

// Report structures for JSON output

#[derive(Debug, Serialize)]
pub struct AddPermissionReport {
    pub permission: String,
    pub status: PermissionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

waterui_cli::impl_report!(AddPermissionReport, |r| {
    match r.status {
        PermissionStatus::Added => format!("Added permission: {}", r.permission),
        PermissionStatus::AlreadyExists => format!("Permission {} already exists", r.permission),
        PermissionStatus::Removed => format!("Removed permission: {}", r.permission),
        PermissionStatus::NotFound => format!("Permission {} not found", r.permission),
    }
});

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionStatus {
    Added,
    AlreadyExists,
    Removed,
    NotFound,
}

#[derive(Debug, Serialize)]
pub struct ListPermissionsReport {
    pub permissions: Vec<PermissionEntry>,
    pub count: usize,
}

waterui_cli::impl_report!(ListPermissionsReport, |r| {
    if r.permissions.is_empty() {
        "No permissions configured".to_string()
    } else {
        format!("{} permissions configured", r.count)
    }
});

#[derive(Debug, Serialize)]
pub struct PermissionEntry {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub ios_keys: Vec<String>,
    pub android_permissions: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AvailablePermissionsReport {
    pub permissions: Vec<AvailablePermissionEntry>,
}

waterui_cli::impl_report!(AvailablePermissionsReport, |r| {
    format!("{} permissions available", r.permissions.len())
});

#[derive(Debug, Serialize)]
pub struct AvailablePermissionEntry {
    pub name: String,
    pub ios_supported: bool,
    pub android_supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ios_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub android_permission: Option<String>,
}

/// Add a permission to the project.
pub fn add(args: AddPermissionArgs) -> Result<AddPermissionReport> {
    let project_dir = args
        .project
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));

    let mut config = Config::load(&project_dir)?;

    // Validate playground-only
    if config.package.package_type != PackageType::Playground {
        bail!(
            "Permission management is only available for playground projects.\n\n\
             For regular app projects, configure permissions directly in:\n\
             • iOS: Info.plist\n\
             • Android: AndroidManifest.xml"
        );
    }

    // Validate permission name
    let permission: Permission = args.permission.parse().map_err(|_| {
        eyre!(
            "Unknown permission '{}'. Run `water permission available` to see valid options.",
            args.permission
        )
    })?;

    // Check if already exists
    if config.permissions.has(&args.permission) {
        if !output::global_output_format().is_json() {
            ui::warning(format!(
                "Permission '{}' is already configured",
                args.permission
            ));
        }
        return Ok(AddPermissionReport {
            permission: args.permission,
            status: PermissionStatus::AlreadyExists,
            description: None,
            message: Some("Permission is already configured".to_string()),
        });
    }

    // Add to config
    if let Some(ref desc) = args.description {
        config
            .permissions
            .add_with_description(args.permission.clone(), desc.clone());
    } else {
        config.permissions.add(args.permission.clone());
    }

    config.save(&project_dir)?;

    if !output::global_output_format().is_json() {
        ui::success(format!("Added permission: {}", args.permission));

        let mapping = permission.mapping();
        if let Some(ref ios) = mapping.ios {
            let keys: Vec<_> = ios.iter().map(|p| p.info_plist_key).collect();
            ui::kv("iOS", keys.join(", "));
        }
        if let Some(ref android) = mapping.android {
            let perms: Vec<_> = android
                .iter()
                .map(|p| {
                    p.manifest_permission
                        .strip_prefix("android.permission.")
                        .unwrap_or(p.manifest_permission)
                })
                .collect();
            ui::kv("Android", perms.join(", "));
        }

        ui::newline();
        ui::info("Run `water run` to rebuild with the new permission.");
    }

    Ok(AddPermissionReport {
        permission: args.permission,
        status: PermissionStatus::Added,
        description: args.description,
        message: None,
    })
}

/// Remove a permission from the project.
pub fn remove(args: RemovePermissionArgs) -> Result<AddPermissionReport> {
    let project_dir = args
        .project
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));

    let mut config = Config::load(&project_dir)?;

    if config.package.package_type != PackageType::Playground {
        bail!("Permission management is only available for playground projects.");
    }

    let removed = config.permissions.remove(&args.permission);

    if !removed {
        if !output::global_output_format().is_json() {
            ui::warning(format!(
                "Permission '{}' was not configured",
                args.permission
            ));
        }
        return Ok(AddPermissionReport {
            permission: args.permission,
            status: PermissionStatus::NotFound,
            description: None,
            message: Some("Permission was not configured".to_string()),
        });
    }

    config.save(&project_dir)?;

    if !output::global_output_format().is_json() {
        ui::success(format!("Removed permission: {}", args.permission));
        ui::newline();
        ui::info("Run `water run` to rebuild without this permission.");
    }

    Ok(AddPermissionReport {
        permission: args.permission,
        status: PermissionStatus::Removed,
        description: None,
        message: None,
    })
}

/// List configured permissions for the project.
pub fn list(args: ListPermissionsArgs) -> Result<ListPermissionsReport> {
    let project_dir = args
        .project
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));

    let config = Config::load(&project_dir)?;

    let mut entries = Vec::new();

    for perm_name in config.permissions.all_enabled() {
        if let Ok(permission) = perm_name.parse::<Permission>() {
            let mapping = permission.mapping();
            let description = config.permissions.get_description(&perm_name);

            entries.push(PermissionEntry {
                name: perm_name,
                description,
                ios_keys: mapping
                    .ios
                    .map(|v| v.iter().map(|p| p.info_plist_key.to_string()).collect())
                    .unwrap_or_default(),
                android_permissions: mapping
                    .android
                    .map(|v| {
                        v.iter()
                            .map(|p| p.manifest_permission.to_string())
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }
    }

    let count = entries.len();

    if !output::global_output_format().is_json() {
        if entries.is_empty() {
            ui::info("No permissions configured.");
            ui::newline();
            ui::info("Add permissions with: water permission add <name>");
            ui::info("See available permissions: water permission available");
        } else {
            ui::section("Configured Permissions");
            for entry in &entries {
                println!();
                ui::kv("Permission", &entry.name);
                if let Some(ref desc) = entry.description {
                    ui::kv("Description", desc);
                }
                if !entry.ios_keys.is_empty() {
                    ui::kv("iOS", entry.ios_keys.join(", "));
                }
                if !entry.android_permissions.is_empty() {
                    let android: Vec<_> = entry
                        .android_permissions
                        .iter()
                        .map(|p| {
                            p.strip_prefix("android.permission.")
                                .unwrap_or(p)
                                .to_string()
                        })
                        .collect();
                    ui::kv("Android", android.join(", "));
                }
            }
        }
    }

    Ok(ListPermissionsReport {
        permissions: entries,
        count,
    })
}

/// Show all available permissions.
pub fn available(args: AvailablePermissionsArgs) -> Result<AvailablePermissionsReport> {
    let filter_ios = args.platform.as_deref() == Some("ios");
    let filter_android = args.platform.as_deref() == Some("android");

    let permissions: Vec<AvailablePermissionEntry> = Permission::all()
        .iter()
        .filter_map(|p| {
            let mapping = p.mapping();
            let ios_supported = mapping.ios.is_some();
            let android_supported = mapping.android.is_some();

            // Apply platform filter
            if filter_ios && !ios_supported {
                return None;
            }
            if filter_android && !android_supported {
                return None;
            }

            Some(AvailablePermissionEntry {
                name: p.to_string(),
                ios_supported,
                android_supported,
                ios_key: mapping
                    .ios
                    .as_ref()
                    .and_then(|v| v.first())
                    .map(|p| p.info_plist_key.to_string()),
                android_permission: mapping
                    .android
                    .as_ref()
                    .and_then(|v| v.first())
                    .map(|p| {
                        p.manifest_permission
                            .strip_prefix("android.permission.")
                            .unwrap_or(p.manifest_permission)
                            .to_string()
                    }),
            })
        })
        .collect();

    if !output::global_output_format().is_json() {
        ui::section("Available Permissions");
        println!();
        println!(
            "{:<25} {:<5} {:<8} Key/Permission",
            "Permission", "iOS", "Android"
        );
        println!("{}", "-".repeat(75));
        for entry in &permissions {
            let ios = if entry.ios_supported { "Yes" } else { "-" };
            let android = if entry.android_supported { "Yes" } else { "-" };
            let detail = entry
                .ios_key
                .as_deref()
                .or(entry.android_permission.as_deref())
                .unwrap_or("-");
            println!("{:<25} {:<5} {:<8} {}", entry.name, ios, android, detail);
        }
        println!();
        ui::info("Add a permission: water permission add <name>");
    }

    Ok(AvailablePermissionsReport { permissions })
}
