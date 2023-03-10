use crate::component::{Component, Entity};
use std::any::Any;

struct World {
    entities: Vec<Entity>,
    next_entity: Entity,
    components: Vec<Box<dyn Any>>,
}

#[allow(dead_code)]
impl World {
    fn new() -> World {
        World {
            entities: Vec::new(),
            next_entity: 0,
            components: Vec::new(),
        }
    }

    fn create_entity(&mut self) -> Entity {
        let entity = self.next_entity;
        self.next_entity += 1;
        self.entities.push(entity);
        entity
    }

    fn delete_entity(&mut self, entity: Entity) {
        let index = self.entities.iter().position(|&e| e == entity).unwrap();
        self.entities.swap_remove(index);
        for component in &mut self.components {
            component
                .downcast_mut::<Component<Entity>>()
                .unwrap()
                .remove(entity);
        }
    }

    fn register_component<T: 'static>(&mut self) -> usize {
        self.components.push(Box::<Component<T>>::default());
        self.components.len() - 1
    }

    fn get_component<T: 'static>(&self, component_id: usize) -> &Component<T> {
        self.components[component_id]
            .downcast_ref::<Component<T>>()
            .unwrap()
    }

    fn get_component_mut<T: 'static>(&mut self, component_id: usize) -> &mut Component<T> {
        self.components[component_id]
            .downcast_mut::<Component<T>>()
            .unwrap()
    }

    fn iter_entities(&self) -> std::slice::Iter<Entity> {
        self.entities.iter()
    }
}
