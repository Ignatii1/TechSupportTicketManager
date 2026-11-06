//! Описание и комментарий — текстом: образцы `HttpIntraserviceClient.SelfCheck` (`Text(…)`, `Same(…)`, `LegacyText`) один в
//! один и `WebUtility.HtmlDecode` по правилам .NET.

use std::time::{Duration, Instant};
use ticketboard::intraservice::html::{decode_entities, html_to_text, legacy_text};
use ticketboard::intraservice::parse;

fn text(raw: &str) -> String {
    html_to_text(raw).unwrap_or_else(|| panic!("пусто: {raw:?}"))
}

#[track_caller]
fn same(raw: &str, want: &str, what: &str) {
    assert_eq!(text(raw), want, "{what}");
}

#[test]
fn plain_text_keeps_its_signs() {
    // живой сервер шлёт простой текст, а в нём «<» и «>» — знаки: снятый «тег» унёс бы текст между ними
    same("остаток < 5 шт, заказ > 10 шт", "остаток < 5 шт, заказ > 10 шт", "простой текст со знаками < и >");
    same(
        "0104606056177299215Rbk\"rR!>ncE;BuBoKW<\r\n0104606056177299215dTPSOa:HbJ0CmP*8GHO\r\n01046060561772992<TTCx>9",
        "0104606056177299215Rbk\"rR!>ncE;BuBoKW<\n0104606056177299215dTPSOa:HbJ0CmP*8GHO\n01046060561772992<TTCx>9",
        "коды маркировки целиком",
    );
    same("<div>a &lt; b</div><div>c</div>", "a < b\nc", "html из редактора — теги сняты, сущности раскрыты");
    // тег — только если за именем пробел, «/» или «>», и в одной строке; пробел в конце строки простого текста остаётся
    same(
        "Иванов <i.ivanov@corp.ru> \r\nостаток < 5 шт, заказ > 10 шт",
        "Иванов <i.ivanov@corp.ru> \nостаток < 5 шт, заказ > 10 шт",
        "почта с буквы",
    );
    same("Пользователь Петрова Анна <300@corp.ru>", "Пользователь Петрова Анна <300@corp.ru>", "почта с цифры");
    same(
        "0104606056177299215Rbk<P\r\n0104606056177299215dTPS>Oa",
        "0104606056177299215Rbk<P\n0104606056177299215dTPS>Oa",
        "<P через строку",
    );
    same("ООО &quot;Ромашка&quot; &amp; Ко", "ООО \"Ромашка\" & Ко", "сущности в простом тексте");
    // через разбор истории — тот же путь, что у C#-проверки
    let life = parse::lifetime(r#"[{"Date":"2026-10-12T10:00:00","Editor":"И","Comments":"остаток < 5 шт, заказ > 10 шт"}]"#).unwrap();
    assert_eq!(life.events[0].comment.as_deref(), Some("остаток < 5 шт, заказ > 10 шт"));
    // пусто и одни пробелы — не текст
    assert_eq!((html_to_text(""), html_to_text(" \r\n\t")), (None, None));
}

#[test]
fn html_as_a_browser_shows_it() {
    same("строка1<br>строка2", "строка1\nстрока2", "html из одних br");
    same("<p>остаток < 5 шт, заказ > 10</p>", "остаток < 5 шт, заказ > 10", "html со знаком <");
    same("<P class=MsoNormal>Текст</P><P>ещё</P>", "Текст\nещё", "заглавные теги с атрибутами");
    same(
        "<html><head><title>Письмо</title><style>p.MsoNormal{margin:0cm}</style><!--[if gte mso 9]><xml><o:OfficeDocumentSettings></o:OfficeDocumentSettings></xml><![endif]--></head><body><p class=MsoNormal>Текст<o:p></o:p></p></body></html>",
        "Текст",
        "письмо Outlook",
    );
    same("<table><tr><td>Склад</td><td>5</td></tr><tr><td>Магазин</td><td>7</td></tr></table>", "Склад 5\nМагазин 7", "таблица");
    same(
        "<p class=MsoListParagraph><![if !supportLists]><span>1. </span><![endif]>Перезагрузить<o:p></o:p></p>",
        "1. Перезагрузить",
        "условные блоки Outlook",
    );
    same("<!DOCTYPE html><section>Срок</section><section>завтра</section>", "Срок\nзавтра", "разделы HTML5");
    same("<dl><dt>Срок</dt><dd>завтра</dd></dl>", "Срок\nзавтра", "список определений");
    same("<p class=MsoNormal>Добрый день! Прошу\r\nпомочь</p>", "Добрый день! Прошу помочь", "перенос строки в исходнике html");
    same(
        "<table>\r\n<tr>\r\n <td><p class=MsoNormal>Склад</p></td>\r\n <td><p class=MsoNormal>5</p></td>\r\n</tr>\r\n</table>",
        "Склад 5",
        "таблица Word",
    );
    same("<!-- <style> old --><p>Важный текст</p><style>p{}</style>", "Важный текст", "<style> в комментарии");
    // без абзацев (одни <br> или строчный тег) переводы строк исходника — строки
    same("строка1<br>\r\nстрока2<br />\r\nстрока3", "строка1\nстрока2\nстрока3", "<br> и перевод строки за ним — одна строка");
    same("Строка 1\r\nСтрока 2 <b>важно</b>", "Строка 1\nСтрока 2 важно", "простой текст с тегом <b>");
}

#[test]
fn single_tags_are_not_html() {
    // html узнаётся по паре «открывающий + такой же закрывающий» известного тега (закрывающий — в одной строке), <br> или <img …>
    same(
        "Коды:\r\n010460<p/7z>91EE06<TTCx>92abc\r\n0104601<b>xyz",
        "Коды:\n010460<p/7z>91EE06<TTCx>92abc\n0104601<b>xyz",
        "открывающие «теги» в кодах",
    );
    same("В шаблоне <div class=\"x\"> не закрыт\r\nСтрока 2", "В шаблоне <div class=\"x\"> не закрыт\nСтрока 2", "упомянутый тег");
    same(
        "Ошибка в шаблоне: лишний </td> в строке 5\r\nНажмите <Ctrl>+<F5>, код 0104<p/7z>99",
        "Ошибка в шаблоне: лишний </td> в строке 5\nНажмите <Ctrl>+<F5>, код 0104<p/7z>99",
        "цитата закрывающего тега",
    );
    same("Код <P>x</P\r\n>Oa<TTCx>92", "Код <P>x</P\n>Oa<TTCx>92", "закрывающий через строку");
    same(
        "В шаблоне лишний </td>, а <td> ниже не закрыт\r\nСтрока 2",
        "В шаблоне лишний </td>, а <td> ниже не закрыт\nСтрока 2",
        "закрывающий раньше открывающего",
    );
    // код маркировки в настоящем html — текст: «<p/7z>» не тег и не абзац
    same("<b>Коды</b>\r\n010460<p/7z>91", "Коды\n010460<p/7z>91", "код в html");
    same("<p>Код 0104<pre/7z>91 и\r\nдалее</p><pre>a</pre>", "Код 0104<pre/7z>91 и далее\na", "код с «<pre/» в html");
}

#[test]
fn breaks_blocks_and_images() {
    // <br> с атрибутами (Word, Outlook) — перевод строки и признак html; картинка — html, снята вместе с данными
    same("<p class=MsoNormal>Строка1<br style='mso-special-character:line-break'>Строка2</p>", "Строка1\nСтрока2", "<br> с атрибутами");
    same("Строка1<br clear=all>Строка2", "Строка1\nСтрока2", "<br clear=all>");
    same("Скриншот:\r\n<img src=\"data:image/png;base64,iVBORw0KGgo=\">", "Скриншот:", "<img>");
    // без абзацев перевод строки исходника сразу за блоком — та же строка, а не лишняя пустая
    same("Ошибка:\r\n<pre>trace</pre>\r\nСпасибо", "Ошибка:\ntrace\nСпасибо", "перевод строки за блоком");
    same(
        "Добрый день!<div>Не печатает принтер</div><div>Спасибо</div>",
        "Добрый день!\nНе печатает принтер\nСпасибо",
        "текст перед блоком",
    );
    same("Список:<ul><li>a<li>b</ul>", "Список:\na\nb", "пункты без закрывающих тегов");
    // пустой абзац Word — пустая строка (а не строка из неразрывного пробела)
    same("<p>Текст</p><p>&nbsp;</p><p>Ещё</p>", "Текст\n\nЕщё", "пустой абзац");
    same(
        "<p>Ошибка:</p><pre>System.Exception: boom\r\n   at A.B()\r\n   at C.D()</pre>",
        "Ошибка:\nSystem.Exception: boom\nat A.B()\nat C.D()",
        "<pre>",
    );
    // (?<!\s) у .NET: после «<br>\n» новая строка уже есть — следующий абзац её не добавляет
    // (без абзацев: с ними перевод строки исходника — уже пробел, и запрет не нужен)
    same("a<br>\r\n<pre>b</pre>", "a\nb", "<br>, перевод строки и блок — одна строка");
    same("<tr><td>a</td>  </td>b</tr>", "a b", "закрывающий ячейки сразу за пробелами после другого");
    // три и больше пустых строк — одна пустая; строка из пробелов — пустая
    same("a\r\n\r\n \r\n\t\r\nb", "a\n\nb", "много пустых строк");
}

#[test]
fn edges_of_the_scanners() {
    // закрывающий — только «</имя», с пробелами перед «>» и после пробела; «<xb>» — не закрывающий «b»
    same("Код <b> и <xb>", "Код <b> и <xb>", "открывающий и чужое имя — не html");
    same("<b>важно</b >", "важно", "пробел перед > у закрывающего");
    same("<b>x </b>", "x", "закрывающий после пробела");
    same("<div>x </div>", "x", "закрывающий блока после пробела");
    // в ячейке: «<xtd>» и «</tx>» — не конец ячейки, просто теги
    same("<table><tr><td>a<xtd>b</td></tr></table>", "ab", "<xtd> в ячейке");
    same("<table><tr><td>a</tx>b</td></tr></table>", "ab", "</tx> в ячейке");
    // <img …> — только с пробелом после имени (и неразрывным), оборванное — текст
    let nbsp = char::from_u32(0xA0).unwrap();
    same("a <im", "a <im", "оборванное <im");
    same("Текст <img> код\r\nещё", "Текст <img> код\nещё", "<img> без пробела — не html");
    same(&format!("Фото<img{nbsp}>"), "Фото", "<img> с неразрывным пробелом");
    // два комментария; пробел после <td>; пробелы перед концом ячейки без абзацев
    same("<p>a</p><!-- x --><p>b</p><!-- y --><p>c</p>", "a\nb\nc", "два комментария");
    same("<table><tr><td> <p>Склад</p></td><td>\r\n5</td></tr></table>", "Склад 5", "пробел после <td>");
    same("<tr><td>a  </td><td>b</td></tr>", "a b", "пробелы перед концом ячейки");
    // <br>, пробелы и перевод строки — одна строка; пробел перед <br> — долой (с абзацами и без)
    same("a<br>  \r\nb", "a\nb", "пробелы между <br> и переводом строки");
    same("<p>a <br>b</p>", "a\nb", "пробел перед <br> с абзацами");
    same("a <br>b", "a\nb", "пробел перед <br> без абзацев");
    // «o:p» — тег, «o:5» — нет
    same("<b>x</b> <o:5>", "x <o:5>", "имя после двоеточия — с буквы");
    // пустые строки в простом тексте сводятся от первого перевода строки: пробел перед ними остаётся
    same("a \r\n\r\n\r\nb", "a \n\nb", "пробел перед пустыми строками");
    // прежний разбор: «<ab>» — тег, «br>» без «<» — текст
    assert_eq!(legacy_text("a<ab>c").as_deref(), Some("ac"));
    assert_eq!(legacy_text("xbr>y").as_deref(), Some("xbr>y"));
}

#[test]
fn it_is_linear() {
    // десятки тысяч одиночных «<b>» и чужих «</i>» в кодах маркировки — не минуты
    let codes = "0104606056<b>72</i>99Rbk\r\n".repeat(60_000);
    let watch = Instant::now();
    let plain = text(&codes);
    assert!(plain.starts_with("0104606056<b>72</i>99Rbk\n") && watch.elapsed() < Duration::from_secs(5), "{:?}", watch.elapsed());
    // и разбор html: тысячи незакрытых <title>, <pre> и комментариев среди чужих закрывающих
    let unclosed = format!("<p>a</p>{}", "<title>x</i><pre>y</p><!-- z\r\n".repeat(30_000));
    let watch = Instant::now();
    let html = text(&unclosed);
    assert!(html.starts_with("a\nx") && watch.elapsed() < Duration::from_secs(5), "{:?}", watch.elapsed());
    // и длинные серии пробелов (лог в <pre>, html без абзацев) и серии <br> через пробел без ячейки за ними
    let spaces = " ".repeat(100_000);
    let watch = Instant::now();
    assert!(text(&format!("<b>x</b>{spaces}y")).ends_with(" y"));
    assert!(text(&format!("<p>a</p><pre>b{spaces}c</pre>")).ends_with(" c"));
    let brs = "<br> ".repeat(50_000);
    assert_eq!(text(&format!("<table><tr><td>a{brs}b</td></tr></table>")), "a\n\nb");
    assert!(watch.elapsed() < Duration::from_secs(5), "{:?}", watch.elapsed());
}

#[test]
fn the_old_parser() {
    // прежний разбор (до 0.15.1) — чтобы узнать на доске описание, им обрезанное
    assert_eq!(legacy_text("остаток < 5 шт, заказ > 10 шт").as_deref(), Some("остаток  10 шт"));
    assert_eq!(legacy_text("<p>a &laquo;b&raquo;</p>").as_deref(), Some("a «b»"));
    assert_eq!(legacy_text("  "), None);
    assert_eq!(legacy_text("a<BR/>b</LI>\r\n\r\n\r\nc").as_deref(), Some("a\nb\n\nc"));
}

#[test]
fn entities_as_dotnet_decodes_them() {
    // имена — с учётом регистра; &apos; — тоже (его нет в HTML 4, но .NET знает)
    assert_eq!(decode_entities("&Auml;&auml;&euro;&hellip;&apos;&AMP;"), "Ää€…'&AMP;");
    // числа: десятичное (пробелы и знак по краям — как uint.TryParse), шестнадцатеричное (без них), вне Unicode и суррогаты — как есть
    assert_eq!(decode_entities("&#1046;&#x416;&#X416;&# 65 ;&#+66;"), "ЖЖЖAB");
    assert_eq!(
        decode_entities("&#x 41;&#xD800;&#55296;&#x110000;&#99999999999;&#;&#x;"),
        "&#x 41;&#xD800;&#55296;&#x110000;&#99999999999;&#;&#x;"
    );
    assert_eq!(decode_entities("&#x1F600;"), char::from_u32(0x1F600).unwrap().to_string());
    // «&» без «;» до следующего «&» — не сущность, а следующий «&» может быть ею
    assert_eq!(decode_entities("a & b &amp; c"), "a & b & c");
    assert_eq!(decode_entities("&&amp;&nosuch;&"), "&&&nosuch;&");
    // знак у шестнадцатеричного — нельзя; у десятичного — один, и минус — только у нуля (uint)
    assert_eq!(decode_entities("&#x+41;&#-65;&#++65;"), "&#x+41;&#-65;&#++65;");
    assert_eq!(decode_entities("&#-0;"), "\0");
    // неразрывный пробел — символом, не пробелом
    assert_eq!(decode_entities("&nbsp;"), char::from_u32(0xA0).unwrap().to_string());
}
