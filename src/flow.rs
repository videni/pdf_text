use crate::classify::{classify, Class};
use crate::node::{Node, NodeTag};
use crate::util::avg;
use crate::text::concat_text;
use std::iter::once;
use pathfinder_geometry::rect::RectF;
use pdf_render::TextSpan;

use std::mem::take;
use font::Encoder;
use serde::{Serialize, Deserialize};
use table::Table;

#[derive(Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    pub rect: Rect,
    pub chars: Vec<Char>
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Char {
    // Byte offset
    pub offset: usize,
    pub pos: f32,
    pub width: f32,
}

#[derive(Serialize, Deserialize)]
pub struct Line {
    pub words: Vec<Word>,
    pub rect: Rect,
}
#[derive(Serialize, Deserialize)]
pub struct Run {
    pub lines: Vec<Line>,
    pub kind: RunType,
}

#[derive(Serialize, Deserialize)]
pub enum RunType {
    ParagraphContinuation,
    Paragraph,
    Header,
    Cell,
}


#[derive(Copy, Clone, Debug)]
#[derive(Serialize, Deserialize)]
#[repr(C)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32
}
impl From<RectF> for Rect {
    fn from(r: RectF) -> Self {
        Rect {
            x: r.origin_x(),
            y: r.origin_y(),
            w: r.width(),
            h: r.height()
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CellContent {
    pub text: String,
    pub rect: Rect,
}

#[derive(Serialize, Deserialize)]
pub struct Flow {
    pub runs: Vec<Run>,
}

impl Flow {
    pub fn new() -> Self {
        Flow { 
            runs: vec![]
        }
    }
    pub fn add_line(&mut self, words: Vec<Word>, kind: RunType, rect: Rect) {
        if words.len() > 0 {
            self.runs.push(Run {
                lines: vec![Line { words, rect}], 
                kind,
            });
        }
    }
    pub fn add_table(&mut self, table: Table<CellContent>) {
        
    }
}

pub(crate) fn build<E: Encoder>(mut flow: &mut Flow, spans: &[TextSpan<E>], node: &Node, x_anchor: f32) {
    match *node {
        Node::Final { ref indices } => {
            if indices.len() > 0 {
                let node_spans = indices.iter()
                    .flat_map(|&i| spans.get(i));
                let bbox = node_spans.clone()
                    .map(|s| s.rect)
                    .reduce(|a, b| a.union_rect(b))
                    .unwrap();
                
                let class = classify(node_spans.clone());
                let mut text = String::new();
                let words = concat_text(&mut text, node_spans);

                let t = match class {
                    Class::Header => RunType::Header,
                    _ => RunType::Paragraph,
                };
              
                flow.add_line(words, t, bbox.into());
            }
        }
        Node::Grid { ref x, ref y, ref cells, tag } => {
            match tag {
                NodeTag::Singleton |
                NodeTag::Line => {
                    let mut indices = vec![];
                    node.indices(&mut indices);

                    let line_spans = indices.iter().flat_map(|&i| spans.get(i));
                    let bbox: RectF = line_spans.clone().map(|s| s.rect).reduce(|a, b| a.union_rect(b)).unwrap().into();

                    let class = classify(line_spans.clone());
                    let mut text = String::new();
                    let words = concat_text(&mut text, line_spans);

                    let t = match class {
                        Class::Header => RunType::Header,
                        _ => RunType::Paragraph,
                    };
                
                    flow.add_line(words, t, bbox.into());
                }
                NodeTag::Paragraph => {
                    assert_eq!(x.len(), 0, "For paragraph x gaps must be empty");

                    let mut lines: Vec<(RectF, usize)> = vec![];
                    let mut indices = vec![];

                    for n in cells {
                        let start: usize = indices.len();
                        n.indices(&mut indices);
                        if indices.len() > start {
                            let cell_spans = indices[start..].iter().flat_map(|&i| spans.get(i));
                            let bbox = cell_spans.map(|s| s.rect).reduce(|a, b| a.union_rect(b)).unwrap().into();
                            lines.push((bbox, indices.len()));
                        }
                    }

                    let para_spans = indices.iter().flat_map(|&i| spans.get(i));
                    let class = classify(para_spans.clone());
                    // the bounding box the paragraph
                    let bbox = lines.iter().map(|t| t.0).reduce(|a, b| a.union_rect(b)).unwrap();
                    let line_height = avg(para_spans.map(|s| s.rect.height())).unwrap();
                    
                    // classify the lines by this vertical line
                    let left_margin = bbox.min_x() + 0.5 * line_height;

                    // count how many are right and left of the split.
                    let mut left = 0;
                    let mut right = 0;

                    for (line_bbox, _) in lines.iter() {
                        if line_bbox.min_x() >= left_margin {
                            right += 1;
                        } else {
                            left += 1;
                        }
                    }
                    //typically paragraphs are indented to the right and longer than 2 lines.
                    //then there will be a higher left count than right count.
                    let indent = left > right;

                    // A paragraph with 3 lines, 3 cases:
                    // case 1: outdented(right > left, will get 3 runs)
                    // |-------
                    // | ----
                    // | ----
                    // case 2: indented (left > right, one new run)
                    // | ------
                    // |-------
                    // |-------
                    // case 3: same x (no indentation, but left > right, right = 0, will be in the same run)
                    // |------
                    // |------
                    // |------

                    //TODO: A paragraph with two lines starts at the same x? then left = right.
                    // the second line will be treated as as another run, but actually it should be in 
                    // in the same run.

                    let mut para_start = 0;
                    let mut line_start = 0;
                    let mut text = String::new();
                    let mut para_bbox = RectF::default();
                    let mut flow_lines = vec![];
                    for &(line_bbox, end) in lines.iter() {
                        if line_start != 0 {
                            //Always add a line break for new line, which will be treated as whitespace in the concat_text method
                            text.push('\n');

                            // if a line is indented(indent = true) or outdented(indent = false), it marks a new paragraph
                            // so here, save previous lines as a new run.
                            if (line_bbox.min_x() >= left_margin) == indent {
                                flow.runs.push(Run {
                                    lines: take(&mut flow_lines),
                                    kind: match class {
                                        Class::Header => RunType::Header,
                                        _ => RunType::Paragraph
                                    },
                                });
                                para_start = line_start;
                            }
                        }
                        if end > line_start {
                            let words = concat_text(&mut text, indices[line_start..end].iter().flat_map(|&i| spans.get(i)));

                            if words.len() > 0 {
                                flow_lines.push(Line { words , rect: line_bbox.into()});
                            }
                        }
                        if para_start == line_start {
                            para_bbox = line_bbox;
                        } else {
                            para_bbox = para_bbox.union_rect(line_bbox);
                        }
                        line_start = end;
                    }

                    flow.runs.push(Run {
                        lines: flow_lines,
                        kind: match class {
                            Class::Header => RunType::Header,
                            _ => RunType::Paragraph
                        }
                    });
                }
                NodeTag::Complex => {
                    let x_anchors = once(x_anchor).chain(x.iter().cloned()).cycle();
                    for (node, x) in cells.iter().zip(x_anchors) {
                        build(flow, spans, node, x);
                    }
                }
            }
        }
        Node::Table { ref table } => {
            if let Some(bbox) = table.values()
                .flat_map(|v| v.value.iter().flat_map(|&i| spans.get(i).map(|s| s.rect)))
                .reduce(|a, b| a.union_rect(b)) {
                let table = table.flat_map(|indices| {
                    if indices.len() == 0 {
                        None
                    } else {
                        let line_spans = indices.iter().flat_map(|&i| spans.get(i));
                        let bbox: RectF = line_spans.clone().map(|s| s.rect).reduce(|a, b| a.union_rect(b)).unwrap().into();

                        let mut text = String::new();
                        concat_text(&mut text, line_spans.clone());
                        Some(CellContent {
                            text,
                            rect: bbox.into(),
                        })
                    }
                });
                flow.add_table(table);
            }
        }
    }
}