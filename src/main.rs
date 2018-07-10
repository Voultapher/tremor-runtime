#[macro_use]
extern crate log;
extern crate env_logger;

extern crate clap;
extern crate futures;
extern crate rand;
extern crate rdkafka;
extern crate rdkafka_sys;

mod classifier;
mod error;
mod grouping;
mod input;
mod limiting;
mod output;
mod parser;
mod pipeline;

use clap::{App, Arg};
use input::Input;
use pipeline::Pipeline;

// consumer example: https://github.com/fede1024/rust-rdkafka/blob/db7cf0883b6086300b7f61998e9fbcfe67cc8e73/examples/at_least_once.rs

fn main() {
    env_logger::init();

    let matches = App::new("traffic shaping utility")
        .version(option_env!("CARGO_PKG_VERSION").unwrap_or(""))
        .about("Simple command line consumer")
        .arg(
            Arg::with_name("input")
                .short("i")
                .long("input")
                .help("input to read from. Valid options are 'stdin' and 'kafka'")
                .takes_value(true)
                .required(true),
        )
        .arg(
            Arg::with_name("input-config")
                .long("input-config")
                .help("Configuration for the input if required.")
                .takes_value(true)
                .default_value(""),
        )
        .arg(
            Arg::with_name("output")
                .short("o")
                .long("output")
                .help("output to send to. Valid options are 'stdout', 'kafka'")
                .takes_value(true)
                .required(true),
        )
        .arg(
            Arg::with_name("output-config")
                .long("output-config")
                .help("Configuration for the output of required.")
                .takes_value(true)
                .default_value(""),
        )
        .arg(
            Arg::with_name("parser")
                .short("p")
                .long("parser")
                .help("parser to use. Valid options are 'raw'")
                .takes_value(true)
                .default_value("raw"),
        )
        .arg(
            Arg::with_name("parser-config")
                .long("parser-config")
                .help("Configuration for the parser if required.")
                .takes_value(true)
                .default_value(""),
        )
        .arg(
            Arg::with_name("classifier")
                .short("c")
                .long("classifier")
                .help("classifier to use. Valid options are 'static'")
                .default_value("static")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("classifier-config")
                .long("classifier-config")
                .help("Configuration for the classifier if required.")
                .takes_value(true)
                .default_value(""),
        )
        .arg(
            Arg::with_name("grouping")
                .short("g")
                .long("grouping")
                .help("grouping logic to use. Valid options are 'drop' and 'pass'")
                .takes_value(true)
                .default_value("pass"),
        )
        .arg(
            Arg::with_name("grouping-config")
                .long("grouping-config")
                .help("Configuration for the grouping.")
                .takes_value(true)
                .default_value(""),
        )
        .arg(
            Arg::with_name("limiting")
                .short("l")
                .long("limiting")
                .help("limiting logic to use. Valid options are 'percentile', 'drop', 'pass'")
                .takes_value(true)
                .default_value("pass"),
        )
        .arg(
            Arg::with_name("limiting-config")
                .long("limiting-config")
                .help("Configuration for the limiter.")
                .takes_value(true)
                .default_value(""),
        )
        .get_matches();

    let input_name = matches.value_of("input").unwrap();
    let input_config = matches.value_of("input-config").unwrap();
    let input = input::new(input_name, input_config);

    let output = matches.value_of("output").unwrap();
    let output_config = matches.value_of("output-config").unwrap();
    let output = output::new(output, output_config);

    let parser = matches.value_of("parser").unwrap();
    let parser_config = matches.value_of("parser-config").unwrap();
    let parser = parser::new(parser, parser_config);

    let classifier = matches.value_of("classifier").unwrap();
    let classifier_config = matches.value_of("classifier-config").unwrap();
    let classifier = classifier::new(classifier, classifier_config);

    let grouping = matches.value_of("grouping").unwrap();
    let grouping_config = matches.value_of("grouping-config").unwrap();
    let grouping = grouping::new(grouping, grouping_config);

    let limiting = matches.value_of("limiting").unwrap();
    let limiting_config = matches.value_of("limiting-config").unwrap();
    let limiting = limiting::new(limiting, limiting_config);

    let pipeline = Pipeline::new(parser, classifier, grouping, limiting, output);

    let _ = input.enter_loop(pipeline);
}
