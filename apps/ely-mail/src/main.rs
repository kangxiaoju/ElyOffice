// Release builds open no console on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ely_mail::mail;
use ely_platform::AppId;

fn main() {
    ely_ui::run(AppId::Mail, mail::open);
}
