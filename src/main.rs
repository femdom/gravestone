#![feature(question_mark)]
#[macro_use(header)]
extern crate hyper;
#[macro_use(log, warn)]
extern crate log;
#[macro_use(row, cell)]
extern crate prettytable;
extern crate backtrace;
extern crate chrono;
extern crate regex;
extern crate select;
extern crate rustc_serialize;
extern crate argparse;

use std::io;
use prettytable::Table;
use prettytable::row::Row;
use prettytable::cell::Cell;
use backtrace::Backtrace;
use argparse::{ArgumentParser, StoreOption};
use chrono::{DateTime, UTC, Local, TimeZone};

use hyper::Client;
use hyper::client::response::Response;
use hyper::header::ContentType;

use regex::Regex;

use rustc_serialize::json::Json;

use select::document::Document;
use select::predicate::{Predicate, Attr, Class, Name};

use std::convert::From;
use std::error::Error as StdError;
use std::fmt;
use std::io::Read;


#[derive(Debug)]
pub enum ErrorType {
    JsonParserError(rustc_serialize::json::ParserError),
    Utf8Error(std::str::Utf8Error),
    HyperError(hyper::Error),
    IoError(std::io::Error),
    TrackingRequestError(hyper::status::StatusCode, Option<Vec<u8>>, Option<String>),
    ProcessResponseFailedError(String),
    HtmlStructureError(String),
    DateTimeParseError(chrono::ParseError),
}

#[derive(Debug)]
pub struct Error {
    bases: Vec<ErrorType>,
    backtrace: Backtrace,
}


impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self.bases.first().expect("Error without bases") {
            ErrorType::HtmlStructureError(ref message) => {
                write!(f, "Wrong HTML document structure: {}", message)
            }
            ErrorType::TrackingRequestError(code, ref content, ref message) => {
                let content = content.as_ref().unwrap();

                write!(f,
                       "{}: {}. {}",
                       code,
                       String::from_utf8_lossy(&content),
                       message.as_ref().unwrap_or(&"None".to_string()))
            }
            _ => write!(f, "{}", self.description()),
        }
    }
}


impl std::error::Error for Error {
    fn description(&self) -> &str {
        match *self.bases.first().expect("Error without bases") {
            ErrorType::TrackingRequestError(_, _, _) => "Request to the tracking service failed",
            ErrorType::ProcessResponseFailedError(_) => {
                "Cannot process response from the tracking service"
            }
            ErrorType::HtmlStructureError(_) => "Unexpected HTML document structure",
            ErrorType::JsonParserError(ref error) => error.description(),
            ErrorType::Utf8Error(ref error) => error.description(),
            ErrorType::IoError(ref error) => error.description(),
            ErrorType::HyperError(ref error) => error.description(),
            ErrorType::DateTimeParseError(ref error) => error.description(),
        }
    }

    fn cause(&self) -> Option<&std::error::Error> {
        match *self.bases.first().expect("Error without bases") {
            ErrorType::JsonParserError(ref error) => Some(error),
            ErrorType::Utf8Error(ref error) => Some(error),
            ErrorType::IoError(ref error) => Some(error),
            ErrorType::HyperError(ref error) => Some(error),
            ErrorType::DateTimeParseError(ref error) => Some(error),
            _ => None,
        }
    }
}


impl Error {
    pub fn caused_by(mut self, error_type: ErrorType) -> Error {
        self.bases.push(error_type);
        return self;
    }

    pub fn new(error_type: ErrorType) -> Error {
        Error {
            bases: vec![error_type],
            backtrace: Backtrace::new(),
        }
    }

    pub fn from_http_response(response: &mut Response, message: Option<String>) -> Error {
        let mut content = Vec::<u8>::new();

        match response.read_to_end(&mut content) {
            Ok(_) => {
                Error::new(ErrorType::TrackingRequestError(response.status, Some(content), message))
            }
            Err(error) => {
                Error::new(ErrorType::IoError(error))
                    .caused_by(ErrorType::TrackingRequestError(response.status, None, message))
            }
        }
    }
}

type Result<T> = std::result::Result<T, Error>;


impl From<rustc_serialize::json::ParserError> for Error {
    fn from(source: rustc_serialize::json::ParserError) -> Error {
        Error::new(ErrorType::JsonParserError(source))
    }
}


impl From<std::str::Utf8Error> for Error {
    fn from(source: std::str::Utf8Error) -> Error {
        Error::new(ErrorType::Utf8Error(source))
    }
}


impl From<hyper::Error> for Error {
    fn from(error: hyper::Error) -> Error {
        Error::new(ErrorType::HyperError(error))
    }
}


impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Error {
        Error::new(ErrorType::IoError(source))
    }
}

impl From<chrono::ParseError> for Error {
    fn from(source: chrono::ParseError) -> Error {
        Error::new(ErrorType::DateTimeParseError(source))
    }
}

pub trait TrackingRetriever {
    fn get_tracking_info(&self, tracking_code: &str) -> Result<Vec<TrackingStatusInfo>>;
}

#[derive(Default, Debug, Eq, PartialEq)]
pub struct TrackingStatusInfo {
    date: Option<DateTime<UTC>>,
    zip_code: Option<String>,
    description: Option<String>,
    status: Option<String>,
    weight: Option<String>,
}

pub struct EMSRussianPostRetriever;
static EMS_RUSSIAN_POST_URL: &'static str = "http://www.emspost.ru/ru/tracking.aspx/getEmsInfo";


#[cfg(test)]
mod test {

    use super::*;
    use chrono::{UTC, TimeZone};

    static CORRECT_DOCUMENT: &'static str = r#"
<table class='emsHeader'>
  <tr>
    <td style='font-weight:bold;'>EMS номер:</td>
    <td>EP011980873RU</td>
  </tr>
  <tr>
    <td style='font-weight:bold;vertical-align:top;'>Принято к пересылке:</td>
    <td>Санкт-Петербург УКД-2<br>
    Отправление EMS Обыкновенное<br>
    Без разряда<br>
    Без отметки</td>
  </tr>
  <tr>
    <td style='font-weight:bold;'>Отправитель:</td>
    <td>КОСТЫЛЕВА</td>
  </tr>
  <tr>
    <td style='font-weight:bold;'>Получатель:</td>
    <td>0</td>
  </tr>
  <tr>
    <td style='font-weight:bold;'>Адресовано:</td>
    <td>423800, Набережные Челны</td>
  </tr>
</table>\r\n\r\n
<table class='emsNumber'>
  <tr>
    <th>Дата</th>
    <th>Почтовый<br>
    индекс</th>
    <th>Описание</th>
    <th>Статус</th>
    <th>Вес<br>
    (кг.)</th>
    <th>Объявл.<br>
    ценность<br>
    (руб.)</th>
    <th>Налож.<br>
    платёж<br>
    (руб.)</th>
  </tr>
  <tr>
    <td nowrap>24.11.2016 16:30</td>
    <td nowrap>190882</td>
    <td nowrap>Санкт-Петербург УКД-2</td>
    <td nowrap>Прием, Единичный</td>
    <td nowrap>1.188</td>
    <td nowrap>-</td>
    <td nowrap>-</td>
  </tr>
  <tr>
    <td nowrap>24.11.2016 21:56</td>
    <td nowrap>190882</td>
    <td nowrap>Санкт-Петербург УКД-2</td>
    <td nowrap>Покинуло сортировочный центр</td>
    <td nowrap>-</td>
    <td nowrap>-</td>
    <td nowrap>-</td>
  </tr>
  <tr>
    <td nowrap>25.11.2016 00:10</td>
    <td nowrap>200994</td>
    <td nowrap>Санкт-Петербург АСЦ EMS</td>
    <td nowrap>Сортировка</td>
    <td nowrap>-</td>
    <td nowrap>-</td>
    <td nowrap>-</td>
  </tr>
</table>
"#;


    #[test]
    fn test_ems_russian_post_retriever_parse_date_should_parse_correct_date() {
        let retriever = EMSRussianPostRetriever;

        assert_eq!(retriever._parse_date("24.08.2016 11:35").unwrap(), UTC.ymd(2016, 08, 24).and_hms(11, 35, 0));
    }

    #[test]
    fn test_ems_russian_post_retriever_parse_table_should_parse_correct_document() {
        let retriever = EMSRussianPostRetriever;

        let result =
            vec![TrackingStatusInfo {
                     date: Some(UTC.ymd(2016, 11, 24).and_hms(16, 30, 0)),
                     zip_code: Some("190882".to_string()),
                     description: Some("Санкт-Петербург УКД-2".to_string()),
                     status: Some("Прием, Единичный".to_string()),
                     weight: Some("1.188".to_string()),
                 },

                 TrackingStatusInfo {
                     date: Some(UTC.ymd(2016, 11, 24).and_hms(21, 56, 0)),
                     zip_code: Some("190882".to_string()),
                     description: Some("Санкт-Петербург УКД-2".to_string()),
                     status: Some("Покинуло сортировочный центр"
                                  .to_string()),
                     weight: Some("-".to_string()),
                 },

                 TrackingStatusInfo {
                     date: Some(UTC.ymd(2016, 11, 25).and_hms(0, 10, 0)),
                     zip_code: Some("200994".to_string()),
                     description: Some("Санкт-Петербург АСЦ EMS".to_string()),
                     status: Some("Сортировка".to_string()),
                     weight: Some("-".to_string()),
                 }];

        assert_eq!(retriever._parse_table(CORRECT_DOCUMENT).unwrap(), result);
    }
}


impl EMSRussianPostRetriever {
    fn _parse_json(&self, response: &Vec<u8>) -> Result<Json> {
        let data = std::str::from_utf8(response)?;
        return Ok(Json::from_str(data)?);
    }

    fn _parse_date(&self, date_str: &str) -> Result<DateTime<UTC>> {
        Ok(UTC.datetime_from_str(date_str, "%d.%m.%Y %H:%M")?)
    }

    fn _parse_table(&self, table_str: &str) -> Result<Vec<TrackingStatusInfo>> {
        let document = Document::from(table_str);

        let table = document.find(Class("emsNumber"))
            .first()
            .ok_or(Error::new(ErrorType::HtmlStructureError("Class not found: \"emsNumber\""
                .to_string())))?;


        let mut result = Vec::<TrackingStatusInfo>::new();

        for row in table.find(Name("tr")).iter().skip(1) {
            let mut status_info = TrackingStatusInfo::default();

            let cells_text: Vec<String> =
                row.find(Name("td")).iter().map(|cell| cell.text()).collect();

            if let Some(date_text) = cells_text.get(0) {

                status_info.date = match self._parse_date(date_text) {
                    Ok(parsed_date) => Some(parsed_date),
                    Err(parse_error) => { warn!("Cannot parse date: {}", parse_error); None },
                };
            } else {
                return Err(Error::new(ErrorType::HtmlStructureError("Cannot get date text from cell".to_string())));
            }

            if let Some(zip_code_text) = cells_text.get(1) {
                status_info.zip_code = Some(zip_code_text.clone());
            } else {
                return Err(Error::new(ErrorType::HtmlStructureError("Cannot get zip code text from cell".to_string())));
            }

            if let Some(description_text) = cells_text.get(2) {
                status_info.description = Some(description_text.clone());
            } else {
                return Err(Error::new(ErrorType::HtmlStructureError("Cannot get description text from cell".to_string())));
            }

            if let Some(status_text) = cells_text.get(3) {
                status_info.status = Some(status_text.clone());
            } else {
                return Err(Error::new(ErrorType::HtmlStructureError("Cannot get status text from cell".to_string())));
            }

            if let Some(weight_text) = cells_text.get(4) {
                status_info.weight = Some(weight_text.clone());
            } else {
                return Err(Error::new(ErrorType::HtmlStructureError("Cannot get weight text from cell".to_string())));
            }

            result.push(status_info);
        }

        return Ok(result);
    }

    fn _process_response(&self, response: &Vec<u8>) -> Result<Vec<TrackingStatusInfo>> {

        let root_json = self._parse_json(response)?;

        let table_str = root_json.as_object()
            .and_then(|root_object| root_object.get("d"))
            .and_then(|data| data.as_string())
            .ok_or(Error::new(ErrorType::ProcessResponseFailedError("Wrong JSON content"
                .to_string())))?;

        return self._parse_table(table_str);
    }

    fn _make_request(&self, tracking_code: &str) -> Result<Vec<u8>> {
        let client = hyper::Client::new();
        let mut response = client.post(EMS_RUSSIAN_POST_URL)
            .body(&format!("{{\"emsNumber\": \"{}\"}}",
                           tracking_code.to_string().to_uppercase()))
            .header(ContentType::json())
            .send()?;

        if response.status == hyper::Ok {
            let mut content = Vec::<u8>::new();
            response.read_to_end(&mut content)?;
            return Ok(content);
        } else {
            return Err(Error::from_http_response(&mut response,
                                                 Some("Cannot get tracking data from EMS \
                                                       Russian Post"
                                                     .to_string())));
        }
    }

}

impl TrackingRetriever for EMSRussianPostRetriever {

    fn get_tracking_info(&self, tracking_code: &str) -> Result<Vec<TrackingStatusInfo>> {
        let response = self._make_request(tracking_code)?;

        return self._process_response(&response);
    }
}


struct Settings {
    tracking_code: Option<String>,
}



fn _parse_arguments() -> Settings {

    let mut settings = Settings {
        tracking_code: None
    };

    {
        let mut parser = ArgumentParser::new();
        parser.set_description("download delivery status information by the tracking code");
        parser.refer(&mut settings.tracking_code)
            .add_option(&["-C", "--tracking-code"], StoreOption, "get tracking information for the given code");

        parser.parse_args_or_exit();
    }

    return settings;
}


fn display_error(error: &Error) {
    println!("Cannot get tracking information");
    println!("{}", error);
}

fn display_tracking_info(tracking_info: Vec<TrackingStatusInfo>) {
    let mut table = Table::new();

    table.set_titles(row!["Date", "ZIP code", "Description", "Status", "Weight"]);

    for line in tracking_info {

        let date_string: String = match line.date {
            Some(date) => format!("{}", date),
            None => "-".to_string(),
        };

        table.add_row(
            Row::new(vec![
                Cell::new(&date_string),
                Cell::new(&line.zip_code.unwrap_or("-".to_string())),
                Cell::new(&line.description.unwrap_or("-".to_string())),
                Cell::new(&line.status.unwrap_or("-".to_string())),
                Cell::new(&line.weight.unwrap_or("-".to_string())),
            ]));
    }
    print!("{}", table);
}


pub fn main() {
    let settings = _parse_arguments();

    match settings.tracking_code {
        Some(code) => {
            EMSRussianPostRetriever.get_tracking_info(&code)
                .map(display_tracking_info);
        },
        _ => {}
    }

}
