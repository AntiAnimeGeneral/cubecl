use crate::matmul::components::global;
use crate::matmul::components::{Ident, MatrixLayout};
use cubecl_core as cubecl;
use cubecl_core::prelude::*;

#[derive(CubeType)]
/// A view of a tensor that starts reading data from a specified offset.
/// Ensures safe access by preventing out-of-bounds errors.
/// Includes pre-fetched shapes and strides for optimized performance.
pub struct TensorView<E: Numeric> {
    pub tensor: Tensor<Line<E>>,
    pub x_offset: u32,
    pub y_offset: u32,
    pub stride_x: u32,
    pub stride_y: u32,
    pub shape_x: u32,
    pub shape_y: u32,
    pub batch_offset: u32,
}

#[cube]
impl<EG: Numeric> TensorView<EG> {
    /// Instanciate a view over the given tensor, pre-fetching needed strides and shapes
    pub fn new(
        tensor: Tensor<Line<EG>>,
        x_offset: u32,
        y_offset: u32,
        nth_batch: u32,
    ) -> TensorView<EG> {
        let rank = tensor.rank();
        let stride_x = tensor.stride(rank - 2);
        let stride_y = tensor.stride(rank - 1);
        let shape_x = tensor.shape(rank - 2);
        let shape_y = tensor.shape(rank - 1);
        let stride_b = tensor.stride(rank - 3);

        TensorView::<EG> {
            tensor,
            x_offset,
            y_offset,
            stride_x,
            stride_y,
            shape_x,
            shape_y,
            batch_offset: nth_batch * stride_b,
        }
    }

    /// Advance the view along the k dimension by a specified offset, `k_offset`.
    pub fn update_view(&mut self, k_offset: u32, #[comptime] ident: Ident) {
        match ident {
            Ident::Lhs => {
                self.y_offset += k_offset;
            }
            Ident::Rhs => {
                self.x_offset += k_offset;
            }
            Ident::Out => {}
        }
    }

    /// Reads data from the tensor view at the specified tile coordinates (tile_x, tile_y).
    ///
    /// Each unit loads one line in a coalesced manner for improved efficiency.
    /// For row-major tensors, subsequent units read lines horizontally within the tile,
    /// while for column-major tensors, they read lines vertically.
    ///
    /// # Note
    ///
    /// Out-of-bounds reads will be translated to zeros.
    pub fn load_coalesced<G: global::Config>(
        &self,
        tile_x: u32,
        tile_y: u32,
        unit_id: u32,
        #[comptime] ident: Ident,
        #[comptime] config: G,
    ) -> Line<EG> {
        let tensor = &self.tensor;
        let line_size = config.line_size(ident);
        let tile_size_x = config.stage_dim(ident).tile_size_x;
        let tile_size_y = config.stage_dim(ident).tile_size_y;

        let view_tile_x = tile_x * tile_size_x + self.x_offset;
        let view_tile_y = tile_y * tile_size_y + self.y_offset;

        let (load_x, load_y) = match config.layout(ident) {
            MatrixLayout::RowMajor => (unit_id / tile_size_y, unit_id % tile_size_y),
            MatrixLayout::ColMajor => (unit_id % tile_size_x, unit_id / tile_size_x),
        };

        let view_x = view_tile_x + load_x;
        let view_y = view_tile_y + load_y;

        let read_pos =
            (view_x * self.stride_x + view_y * self.stride_y + self.batch_offset) / line_size;

        select(
            view_x < self.shape_x && view_y < self.shape_y,
            tensor[read_pos],
            Line::empty(line_size).fill(EG::from_int(0)),
        )
    }

    /// Writes data into the tensor view at the specified coordinates (write_x, write_y).
    ///
    /// Each unit writes one line in a coalesced manner for improved efficiency, assuming row-major layout.
    pub fn write_coalesced<ES: Numeric, G: global::Config>(
        &mut self,
        tile_x: u32,
        tile_y: u32,
        unit_id: u32,
        value: Line<ES>,
        #[comptime] config: G,
    ) {
        let tensor = &mut self.tensor;
        let stage_dim = config.stage_dim(Ident::Out);

        let view_x =
            tile_x * stage_dim.tile_size_x + unit_id / stage_dim.tile_size_y + self.x_offset;
        let view_y =
            tile_y * stage_dim.tile_size_y + unit_id % stage_dim.tile_size_y + self.y_offset;

        let write_position = (view_x * self.stride_x + view_y * self.stride_y + self.batch_offset)
            / tensor.line_size();

        if config.check_m_bounds() {
            if config.check_n_bounds() {
                if view_x < self.shape_x && view_y < self.shape_y {
                    tensor[write_position] = Line::cast_from(value);
                }
            } else if view_x < self.shape_x {
                tensor[write_position] = Line::cast_from(value);
            }
        } else if config.check_n_bounds() {
            if view_y < self.shape_y {
                tensor[write_position] = Line::cast_from(value);
            }
        } else {
            tensor[write_position] = Line::cast_from(value);
        }
    }
}
