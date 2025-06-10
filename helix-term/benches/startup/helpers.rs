use std::{mem::replace, path::PathBuf, time::Duration};

use anyhow::bail;
use crossterm::event::{Event, KeyEvent};
use helix_core::{test, Selection, Transaction};
use helix_term::{application::Application, args::Args, config::Config, keymap::merge_keys};
use helix_view::{current_ref, editor::LspConfig, input::parse_macro};
use tokio_stream::wrappers::UnboundedReceiverStream;

/// Generates a config with defaults more suitable for integration tests
pub fn test_config() -> Config {
    Config {
        editor: test_editor_config(),
        keys: helix_term::keymap::default(),
        ..Default::default()
    }
}

pub fn test_editor_config() -> helix_view::editor::Config {
    helix_view::editor::Config {
        lsp: LspConfig {
            enable: false,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Generates language config loader that merge in overrides, like a user language
/// config. The argument string must be a raw TOML document.
pub fn test_syntax_loader(overrides: Option<String>) -> helix_core::syntax::Loader {
    let mut lang = helix_loader::config::default_lang_config();

    if let Some(overrides) = overrides {
        let override_toml = toml::from_str(&overrides).unwrap();
        lang = helix_loader::merge_toml_values(lang, override_toml, 3);
    }

    helix_core::syntax::Loader::new(lang.try_into().unwrap()).unwrap()
}

pub struct AppBuilder {
    args: Args,
    config: Config,
    syn_loader: helix_core::syntax::Loader,
    input: Option<(String, Selection)>,
}

impl Default for AppBuilder {
    fn default() -> Self {
        Self {
            args: Args::default(),
            config: test_config(),
            syn_loader: test_syntax_loader(None),
            input: None,
        }
    }
}

impl AppBuilder {
    pub fn new() -> Self {
        AppBuilder::default()
    }

    #[allow(dead_code)]
    pub fn with_file<P: Into<PathBuf>>(
        mut self,
        path: P,
        pos: Option<helix_core::Position>,
    ) -> Self {
        self.args
            .files
            .insert(path.into(), vec![pos.unwrap_or_default()]);

        self
    }

    // Remove this attribute once `with_config` is used in a test:
    #[allow(dead_code)]
    pub fn with_config(mut self, mut config: Config) -> Self {
        let keys = replace(&mut config.keys, helix_term::keymap::default());
        merge_keys(&mut config.keys, keys);
        self.config = config;
        self
    }

    #[allow(dead_code)]
    pub fn with_input_text<S: Into<String>>(mut self, input_text: S) -> Self {
        self.input = Some(test::print(&input_text.into()));
        self
    }

    #[allow(dead_code)]
    pub fn with_lang_loader(mut self, syn_loader: helix_core::syntax::Loader) -> Self {
        self.syn_loader = syn_loader;
        self
    }

    pub fn build(self) -> anyhow::Result<Application> {
        if let Some(path) = &self.args.working_directory {
            bail!("Changing the working directory to {path:?} is not yet supported for integration tests");
        }

        if let Some((path, _)) = self.args.files.first().filter(|p| p.0.is_dir()) {
            bail!("Having the directory {path:?} in args.files[0] is not yet supported for integration tests");
        }

        let mut app = Application::new(self.args, self.config, self.syn_loader)?;

        if let Some((text, selection)) = self.input {
            let (view, doc) = helix_view::current!(app.editor);
            let sel = doc.selection(view.id).clone();
            let trans = Transaction::change_by_selection(doc.text(), &sel, |_| {
                (0, doc.text().len_chars(), Some((text.clone()).into()))
            })
            .with_selection(selection);

            // replace the initial text with the input text
            doc.apply(&trans, view.id);
        }

        Ok(app)
    }
}

#[inline]
pub async fn test_key_sequence(
    app: &mut Application,
    in_keys: Option<&str>,
    test_fn: Option<&dyn Fn(&Application)>,
    should_exit: bool,
) -> anyhow::Result<()> {
    test_key_sequences(app, vec![(in_keys, test_fn)], should_exit).await
}

#[allow(clippy::type_complexity)]
pub async fn test_key_sequences(
    app: &mut Application,
    inputs: Vec<(Option<&str>, Option<&dyn Fn(&Application)>)>,
    should_exit: bool,
) -> anyhow::Result<()> {
    const TIMEOUT: Duration = Duration::from_millis(500);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut rx_stream = UnboundedReceiverStream::new(rx);
    let num_inputs = inputs.len();

    for (i, (in_keys, test_fn)) in inputs.into_iter().enumerate() {
        let (view, doc) = current_ref!(app.editor);
        let state = test::plain(doc.text().slice(..), doc.selection(view.id));

        log::debug!("executing test with document state:\n\n-----\n\n{}", state);

        if let Some(in_keys) = in_keys {
            for key_event in parse_macro(in_keys)?.into_iter() {
                let key = Event::Key(KeyEvent::from(key_event));
                log::trace!("sending key: {:?}", key);
                tx.send(Ok(key))?;
            }
        }

        let app_exited = !app.event_loop_until_idle(&mut rx_stream).await;

        if !app_exited {
            let (view, doc) = current_ref!(app.editor);
            let state = test::plain(doc.text().slice(..), doc.selection(view.id));

            log::debug!(
                "finished running test with document state:\n\n-----\n\n{}",
                state
            );
        }

        // the app should not exit from any test until the last one
        if i < num_inputs - 1 && app_exited {
            bail!("application exited before test function could run");
        }

        // verify if it exited on the last iteration if it should have and
        // the inverse
        if i == num_inputs - 1 && app_exited != should_exit {
            bail!("expected app to exit: {} != {}", should_exit, app_exited);
        }

        if let Some(test) = test_fn {
            test(app);
        };
    }

    if !should_exit {
        for key_event in parse_macro("<esc>:q!<ret>")?.into_iter() {
            tx.send(Ok(Event::Key(KeyEvent::from(key_event))))?;
        }

        let event_loop = app.event_loop(&mut rx_stream);
        tokio::time::timeout(TIMEOUT, event_loop).await?;
    }

    let errs = app.close().await;

    if !errs.is_empty() {
        log::error!("Errors closing app");

        for err in errs {
            log::error!("{}", err);
        }

        bail!("Error closing app");
    }

    Ok(())
}
