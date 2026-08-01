#![allow(clippy::unwrap_used)]
#![cfg(test)]
mod tests {
    use anyhow::Result;

    use crate::libraries::events::EventManager;
    use crate::shared::blocks::Block;
    use crate::shared::blocks::BlockChangeEvent;
    use crate::shared::blocks::Blocks;
    use crate::shared::blocks::Tool;
    use crate::shared::blocks::ToolId;

    #[test]
    fn test_blocks_new() {
        let blocks = Blocks::new();
        assert_eq!(blocks.get_size(), (0, 0));
    }

    #[test]
    fn test_blocks_create_dimensions() {
        let mut blocks = Blocks::new();
        blocks.create((42, 50));
        assert_eq!(blocks.get_size(), (42, 50));
    }

    #[test]
    fn test_blocks_create_dimensions_twice() {
        let mut blocks = Blocks::new();
        blocks.create((42, 50));
        blocks.create((10, 11));
        assert_eq!(blocks.get_size(), (10, 11));
    }

    fn assert_ok_and_eq<T: PartialEq>(result: Result<T>, expected: &T) {
        assert!(result.unwrap() == *expected);
    }

    #[test]
    fn test_blocks_set_get() {
        let mut blocks = Blocks::new();
        blocks.create((50, 50));
        let mut block_type1 = Block::new();
        block_type1.name = "test_block_1".to_owned();
        let mut block_type2 = Block::new();
        block_type2.name = "test_block_2".to_owned();
        let block_id1 = blocks.register_new_block_type(block_type1).unwrap();
        let block_id2 = blocks.register_new_block_type(block_type2).unwrap();

        let mut events = EventManager::new();

        blocks.set_block(&mut events, 0, 0, block_id1).unwrap();
        blocks.set_block(&mut events, 1, 0, block_id2).unwrap();
        blocks.set_block(&mut events, 0, 1, block_id1).unwrap();
        blocks.set_block(&mut events, 1, 1, block_id2).unwrap();
        assert_ok_and_eq(blocks.get_block(0, 0), &block_id1);
        assert_ok_and_eq(blocks.get_block(1, 0), &block_id2);
        assert_ok_and_eq(blocks.get_block(0, 1), &block_id1);
        assert_ok_and_eq(blocks.get_block(1, 1), &block_id2);
        assert_ok_and_eq(blocks.get_block(2, 2), &blocks.air());
    }

    #[test]
    fn test_blocks_set_out_of_bound() {
        let mut blocks = Blocks::new();
        blocks.create((50, 50));
        let mut block_type1 = Block::new();
        block_type1.name = "test_block_1".to_owned();
        let block_id1 = blocks.register_new_block_type(block_type1).unwrap();

        let mut events = EventManager::new();

        assert!(blocks.set_block(&mut events, 50, 50, block_id1).is_err());
        assert!(blocks.set_block(&mut events, 51, 51, block_id1).is_err());
        assert!(blocks.set_block(&mut events, 52, 52, block_id1).is_err());
        assert!(blocks.set_block(&mut events, 100, 100, block_id1).is_err());
        assert!(blocks.set_block(&mut events, -1, -1, block_id1).is_err());
        assert!(blocks.set_block(&mut events, -100, 5, block_id1).is_err());
        assert!(blocks.set_block(&mut events, 5, -100, block_id1).is_err());
        assert!(blocks.set_block(&mut events, 2, 1000, block_id1).is_err());
        assert!(blocks.set_block(&mut events, 1000, 2, block_id1).is_err());
    }

    #[test]
    fn test_blocks_create_from_block_ids() {
        let mut blocks = Blocks::new();

        let mut block_type1 = Block::new();
        block_type1.name = "test_block_1".to_owned();
        let mut block_type2 = Block::new();
        block_type2.name = "test_block_2".to_owned();
        let block_id1 = blocks.register_new_block_type(block_type1).unwrap();
        let block_id2 = blocks.register_new_block_type(block_type2).unwrap();

        let blocks_vector = vec![
            vec![block_id1, block_id1, block_id1],
            vec![block_id1, block_id2, block_id2],
            vec![block_id1, block_id2, block_id1],
            vec![block_id1, block_id2, block_id2],
        ];
        blocks.create_from_block_ids(&blocks_vector).unwrap();

        assert_eq!(blocks.get_size(), (4, 3));
        assert_ok_and_eq(blocks.get_block(0, 0), &block_id1);
        assert_ok_and_eq(blocks.get_block(0, 1), &block_id1);
        assert_ok_and_eq(blocks.get_block(0, 2), &block_id1);
        assert_ok_and_eq(blocks.get_block(1, 0), &block_id1);
        assert_ok_and_eq(blocks.get_block(1, 1), &block_id2);
        assert_ok_and_eq(blocks.get_block(1, 2), &block_id2);
        assert_ok_and_eq(blocks.get_block(2, 0), &block_id1);
        assert_ok_and_eq(blocks.get_block(2, 1), &block_id2);
        assert_ok_and_eq(blocks.get_block(2, 2), &block_id1);
        assert_ok_and_eq(blocks.get_block(3, 0), &block_id1);
        assert_ok_and_eq(blocks.get_block(3, 1), &block_id2);
        assert_ok_and_eq(blocks.get_block(3, 2), &block_id2);
    }

    #[test]
    fn test_set_spawns_event() {
        let mut blocks = Blocks::new();
        blocks.create((50, 50));
        let mut block_type1 = Block::new();
        block_type1.name = "test_block_1".to_owned();
        let mut block_type2 = Block::new();
        block_type2.name = "test_block_2".to_owned();
        let block_id1 = blocks.register_new_block_type(block_type1).unwrap();
        let block_id2 = blocks.register_new_block_type(block_type2).unwrap();

        let mut events = EventManager::new();

        blocks.set_block(&mut events, 0, 0, block_id1).unwrap();
        blocks.set_block(&mut events, 2, 1, block_id1).unwrap();
        blocks.set_block(&mut events, 3, 3, block_id1).unwrap();
        blocks.set_block(&mut events, 3, 3, block_id1).unwrap();
        blocks.set_block(&mut events, 3, 3, block_id2).unwrap();

        let event = events.pop_event().unwrap();
        let event = event.downcast::<BlockChangeEvent>().unwrap();
        assert_eq!(event.x, 0);
        assert_eq!(event.y, 0);
        assert!(event.prev_block == blocks.air());

        let event = events.pop_event().unwrap();
        let event = event.downcast::<BlockChangeEvent>().unwrap();
        assert_eq!(event.x, 2);
        assert_eq!(event.y, 1);
        assert!(event.prev_block == blocks.air());

        let event = events.pop_event().unwrap();
        let event = event.downcast::<BlockChangeEvent>().unwrap();
        assert_eq!(event.x, 3);
        assert_eq!(event.y, 3);
        assert!(event.prev_block == blocks.air());

        let event = events.pop_event().unwrap();
        let event = event.downcast::<BlockChangeEvent>().unwrap();
        assert_eq!(event.x, 3);
        assert_eq!(event.y, 3);
        assert!(event.prev_block == block_id1);

        let event = events.pop_event();
        assert!(event.is_none());
    }

    fn new_test_block(name: &str) -> Block {
        let mut block = Block::new();
        block.name = name.to_owned();
        block
    }

    #[test]
    fn test_register_block_empty_name_rejected() {
        let mut blocks = Blocks::new();
        let block = new_test_block("");
        assert!(blocks.register_new_block_type(block).is_err());
    }

    #[test]
    fn test_register_block_duplicate_name_rejected() {
        let mut blocks = Blocks::new();
        assert!(blocks.register_new_block_type(new_test_block("dirt")).is_ok());
        assert!(blocks.register_new_block_type(new_test_block("dirt")).is_err());
        assert!(blocks.register_new_block_type(new_test_block("stone")).is_ok());
    }

    #[test]
    fn test_register_block_unknown_tool_rejected() {
        let mut blocks = Blocks::new();
        // a tool id that has never been registered is rejected
        let mut block = new_test_block("weird");
        block.effective_tool = Some(ToolId::new());
        assert!(blocks.register_new_block_type(block).is_err());
    }

    #[test]
    fn test_register_block_known_tool_accepted() {
        let mut blocks = Blocks::new();
        assert!(blocks.register_new_tool_type(Tool { name: "pickaxe".to_owned(), id: ToolId::new() }).is_ok());

        let mut block = new_test_block("stone_block");
        block.effective_tool = Some(blocks.get_tool_id_by_name(&"pickaxe".to_owned()).unwrap());
        assert!(blocks.register_new_block_type(block).is_ok());
    }

    #[test]
    fn test_register_tool_duplicate_name_rejected() {
        let mut blocks = Blocks::new();
        assert!(blocks.register_new_tool_type(Tool { name: "pickaxe".to_owned(), id: ToolId::new() }).is_ok());
        assert!(blocks.register_new_tool_type(Tool { name: "pickaxe".to_owned(), id: ToolId::new() }).is_err());
    }
}
