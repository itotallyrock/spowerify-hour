#![feature(drain_filter)]

use std::fs;
use std::io::{stdin, stdout, Write};
use std::time::Duration;

use aspotify::{Client, ClientCredentials, CountryCode, Market, Play, PlaylistItem, PlaylistItemType, PlaylistSimplified, Scope, Track, UserPrivate};
use fastrand::shuffle;
use itertools::Itertools;

const MINIMUM_PLAYLIST_LENGTH: usize = 60;
const MINIMUM_SONG_LENGTH: Duration = Duration::from_secs(60);
const SPOTIFY_PLAYLIST_TRACK_FETCH_LIMIT: usize = 64;
const DEFAULT_MARKET: Market = Market::Country(CountryCode::USA);
const REQUIRED_SCOPES: [Scope; 8] = [
    Scope::UserReadPrivate,
    Scope::PlaylistReadPrivate,
    Scope::PlaylistReadCollaborative,
    Scope::AppRemoteControl,
    Scope::UserReadPlaybackState,
    Scope::UserModifyPlaybackState,
    Scope::UserReadCurrentlyPlaying,
    Scope::UserReadPlaybackPosition,
];

#[tokio::main]
async fn main() {
    let credentials = ClientCredentials::from_env()
        .expect("CLIENT_ID and CLIENT_SECRET not found.");
    let client = create_client(credentials).await;
    let power_hour_tracks = choose_playlist(&client).await.unwrap();
    power_hour_tracks.iter().for_each(|t| println!("{} - {}", t.name, t.artists.iter().map(|a| a.name.clone()).join(", ")));
    // TODO: Skip remainder of songs after 1 minute
    play_tracks(&client, power_hour_tracks).await;
}

async fn play_tracks(client: &Client, tracks: Vec<Track>) {
    client.player().play(Some(Play::Tracks(tracks.iter().filter_map(|t| t.id.as_ref()))), None, None).await.expect("failed to play track");
}

async fn create_client(credentials: ClientCredentials) -> Client {
    let mut client = Client::new(credentials.clone());
    if let Ok(refresh_token) = fs::read_to_string(".refresh_token") {
        client = Client::with_refresh(credentials, refresh_token);
    } else {
        authorize(&client).await;
    }

    client
}

async fn authorize(client: &Client) {
    let (url, state) = aspotify::authorization_url(&client.credentials.id, REQUIRED_SCOPES.iter().copied(), false, "http://localhost:8888/spowerify-auth-callback");

    println!("Login via {}", url);

    // Receive the URL that was redirected to.
    print!("Enter the URL that you were redirected to: ");
    stdout().flush().unwrap();
    let mut redirect = String::new();
    stdin().read_line(&mut redirect).unwrap();

    // Create the refresh token from the redirected URL.
    client.redirected(&redirect, &state).await.unwrap();

    // Put the refresh token in a file.
    let refresh_token = client.refresh_token().await.unwrap();
    fs::write(".refresh_token", refresh_token).unwrap();
}

async fn get_current_user(client: &Client) -> UserPrivate {
    client.users_profile().get_current_user().await.expect("failed to get current user").data
}

async fn get_users_playlists(client: &Client, user_id: &str) -> Vec<PlaylistSimplified> {
    let users_playlists = client.playlists().get_users_playlists(&user_id, 30, 0).await.expect("failed to read users playlists").data.items;

    users_playlists.into_iter().filter(|p| p.tracks.total >= MINIMUM_PLAYLIST_LENGTH).collect()
}

async fn choose_playlist(client: &Client) -> Result<Vec<Track>, aspotify::Error> {
    let current_user = get_current_user(client).await;
    let user_market = DEFAULT_MARKET; // TODO: Maybe get from user

    let valid_playlists = get_users_playlists(client, &current_user.id).await;

    for (index, users_playlist) in valid_playlists.iter().enumerate() {
        println!("> {}. {} ({})", index + 1, users_playlist.name, users_playlist.tracks.total);
    }

    let mut chosen_string_index = String::new();
    stdin().read_line(&mut chosen_string_index).unwrap();
    let chosen_index = chosen_string_index.trim().parse::<usize>().expect("invalid index");

    let chosen_playlist = &valid_playlists[chosen_index - 1];
    println!("Using {}", chosen_playlist.name);
    // TODO: Make multiple requests to get a all of the songs in a playlist instead of just the first PLAYLIST_LIMIT songs
    // let mut chosen_playlist_songs = client.playlists().get_playlists_items(chosen_playlist.id.as_str(), PLAYLIST_LIMIT, 0, Some(user_market)).await?.data.items;
    let mut chosen_playlist_songs = read_playlist_items(client, chosen_playlist.id.as_str(), Some(user_market)).await;

    println!("Choosing 60, 1 minute segments out of {} songs in playlist\n", chosen_playlist_songs.len());

    chosen_playlist_songs.drain_filter(|playlist_item| playlist_item.is_local || playlist_item.item.is_none() || match playlist_item.item.as_ref().unwrap() {
        PlaylistItemType::Track(track) => track.duration < MINIMUM_SONG_LENGTH || track.is_playable.map_or(true, |is_playable| !is_playable),
        PlaylistItemType::Episode(_) => true,
    });

    println!("{} valid songs", chosen_playlist_songs.len());
    chosen_playlist_songs.truncate(MINIMUM_PLAYLIST_LENGTH);

    Ok(chosen_playlist_songs.into_iter().filter_map(|playlist_item| match playlist_item.item.unwrap() {
        PlaylistItemType::Track(track) => Some(track),
        PlaylistItemType::Episode(_) => None,
    }).collect())
}

async fn read_playlist_items(client: &Client, playlist_id: &str, user_market: Option<Market>) -> Vec<PlaylistItem> {
    let mut playlist_items = Vec::with_capacity(500);
    let mut offset = 0;
    loop {
        let mut chunked_items = client.playlists().get_playlists_items(playlist_id, SPOTIFY_PLAYLIST_TRACK_FETCH_LIMIT, offset, user_market).await.expect("failed to read playlist items").data.items;
        if chunked_items.is_empty() {
            break;
        }
        offset += chunked_items.len();
        playlist_items.append(&mut chunked_items);
    }

    shuffle(&mut playlist_items);

    playlist_items
}
